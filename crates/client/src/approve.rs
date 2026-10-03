//! Approving a new device from an existing one, as an alternative to typing
//! the password on it (ADR 0004).
//!
//! One device shows a link, also as a QR code, and the other scans it or has
//! it pasted. Either can show it, whichever way the cameras point:
//!
//! - the existing device shows it (Settings → Account), and the new device
//!   scans it ([`NewDevice::scan`]); or
//! - the new device shows it ([`NewDevice::show`]), and the existing device
//!   scans it.
//!
//! The link carries the server's address and certificate fingerprint, the
//! approval's id, and a random key made by the device that shows it. Over the
//! server the new device sends its name and keys, and the existing device
//! answers with the account's keys, all sealed under that key, which the
//! server never sees (see `hab_proto::approval`). The existing device shows
//! the new device's name and asks for confirmation before it approves.

use crate::code::{decode, encode, plausible_fingerprint};
use crate::keystore::{KeyId, KeyKind, KeyStore, KeyStoreError};
use crate::profile::Profile;
use crate::signin::SignedIn;
use crate::tls::{parse_address, Pinned, TlsError};
use hab_proto::wire::{
    ApprovalCollect, ApprovalCollected, ApprovalGrantPlain, ApprovalOpen, ApprovalRef,
    ApprovalRequest, ApprovalRequestPlain,
};
use hab_proto::{verify_device, ApprovalKey, DeviceKeys, KeyError, Keys};
use rand::RngCore;
use std::time::Duration;

pub(crate) const APPROVE_SCHEME: &str = "hab-bot://approve";

#[derive(Debug, thiserror::Error)]
pub enum ApproveError {
    #[error("{0}")]
    Input(&'static str),
    #[error("That isn't an approval link.")]
    BadLink,
    #[error("This approval has expired or was already used. Start again.")]
    Expired,
    #[error("The other device did not approve this one in time. Start again.")]
    TimedOut,
    #[error("The approval did not check out: {0}")]
    Invalid(&'static str),
    #[error(transparent)]
    Server(#[from] TlsError),
    #[error(transparent)]
    Keys(#[from] KeyError),
    #[error(transparent)]
    Store(#[from] KeyStoreError),
}

/// What the server answers when an approval is unknown, used up or expired.
pub(crate) fn expired_or(e: TlsError) -> ApproveError {
    match e {
        TlsError::Status { status: 404, .. } => ApproveError::Expired,
        e => ApproveError::Server(e),
    }
}

/// The link, or QR code, one device shows to the other.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApprovalLink {
    pub address: String,
    pub server_name: String,
    /// `AA:BB:..`, SHA-256 of the server's certificate.
    pub fingerprint: String,
    pub id: String,
    pub key: ApprovalKey,
}

impl ApprovalLink {
    /// The link, for copying or opening on the other device.
    pub fn link(&self) -> String {
        format!(
            "{APPROVE_SCHEME}?address={}&name={}&fingerprint={}&id={}&key={}",
            encode(&self.address),
            encode(&self.server_name),
            encode(&self.fingerprint),
            encode(&self.id),
            self.key.to_text()
        )
    }

    /// The link as a QR code, an SVG image.
    pub fn qr_svg(&self) -> String {
        let code = qrcode::QrCode::new(self.link().as_bytes()).expect("a link fits in a QR code");
        code.render::<qrcode::render::svg::Color>()
            .min_dimensions(240, 240)
            .quiet_zone(true)
            .build()
    }

    pub fn parse(text: &str) -> Result<ApprovalLink, crate::code::CodeError> {
        use crate::code::CodeError::Invalid;
        let query = text
            .trim()
            .strip_prefix(APPROVE_SCHEME)
            .and_then(|rest| rest.strip_prefix('?'))
            .ok_or(Invalid)?;
        let (mut address, mut name, mut fingerprint, mut id, mut key) =
            (None, None, None, None, None);
        for pair in query.split('&') {
            let (k, value) = pair.split_once('=').ok_or(Invalid)?;
            let slot = match k {
                "address" => &mut address,
                "name" => &mut name,
                "fingerprint" => &mut fingerprint,
                "id" => &mut id,
                "key" => &mut key,
                _ => continue,
            };
            *slot = Some(decode(value).ok_or(Invalid)?);
        }
        let (Some(address), Some(server_name), Some(fingerprint), Some(id), Some(key)) =
            (address, name, fingerprint, id, key)
        else {
            return Err(Invalid);
        };
        let plausible_id = id.len() == 32 && id.chars().all(|c| c.is_ascii_hexdigit());
        if parse_address(&address).is_err() || !plausible_fingerprint(&fingerprint) || !plausible_id
        {
            return Err(Invalid);
        }
        Ok(ApprovalLink {
            address,
            server_name,
            fingerprint,
            id,
            key: ApprovalKey::from_text(&key).map_err(|_| Invalid)?,
        })
    }

    pub(crate) fn server(&self) -> Pinned {
        Pinned::new(&self.address, &self.fingerprint)
    }
}

/// A device that is being approved, before it has been. It has its own keys
/// and has asked the server to relay its request; nothing is stored until the
/// answer arrives.
pub struct NewDevice {
    link: ApprovalLink,
    collect_token: String,
    keys: DeviceKeys,
    device_name: String,
    portable: bool,
}

/// How often to ask the server whether the existing device has approved.
const POLL: Duration = Duration::from_millis(500);

fn request_of(
    keys: &DeviceKeys,
    name: &str,
    portable: bool,
    id: &str,
    key: &ApprovalKey,
) -> hab_proto::wire::ApprovalBlob {
    let plain = ApprovalRequestPlain {
        name: name.to_string(),
        portable,
        signing_public: keys.signing.verifying_key().to_bytes().to_vec(),
        sealing_public: keys.sealing_public().to_vec(),
    };
    key.seal(
        id,
        "request",
        &serde_json::to_vec(&plain).expect("a request serializes"),
    )
}

fn new_token() -> String {
    let mut bytes = [0u8; 32];
    rand::rngs::OsRng.fill_bytes(&mut bytes);
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn check_name(name: &str) -> Result<String, ApproveError> {
    let name = name.trim();
    if name.is_empty() {
        return Err(ApproveError::Input("This device needs a name."));
    }
    Ok(name.to_string())
}

impl NewDevice {
    /// This device shows the code: it starts an approval at the server, and
    /// the existing device scans [`NewDevice::link`] or has it pasted.
    pub async fn show(
        address: &str,
        fingerprint: &str,
        server_name: &str,
        device_name: &str,
        portable: bool,
    ) -> Result<NewDevice, ApproveError> {
        let device_name = check_name(device_name)?;
        let link = ApprovalLink {
            address: address.to_string(),
            server_name: server_name.to_string(),
            fingerprint: fingerprint.to_string(),
            id: new_token()[..32].to_string(),
            key: ApprovalKey::generate(),
        };
        let keys = DeviceKeys::generate();
        let collect_token = new_token();
        let _: ApprovalRef = link
            .server()
            .post(
                "/api/v1/approvals/open",
                &ApprovalOpen {
                    approval_id: link.id.clone(),
                    request: request_of(&keys, &device_name, portable, &link.id, &link.key),
                    collect_token: collect_token.clone(),
                },
            )
            .await?;
        Ok(NewDevice {
            link,
            collect_token,
            keys,
            device_name,
            portable,
        })
    }

    /// This device scanned (or was given) an existing device's code: it sends
    /// its name and keys for the existing device to confirm.
    pub async fn scan(
        link: &ApprovalLink,
        device_name: &str,
        portable: bool,
    ) -> Result<NewDevice, ApproveError> {
        let device_name = check_name(device_name)?;
        let keys = DeviceKeys::generate();
        let collect_token = new_token();
        let sent: Result<serde_json::Value, TlsError> = link
            .server()
            .post(
                "/api/v1/approvals/request",
                &ApprovalRequest {
                    approval_id: link.id.clone(),
                    request: request_of(&keys, &device_name, portable, &link.id, &link.key),
                    collect_token: collect_token.clone(),
                },
            )
            .await;
        sent.map_err(expired_or)?;
        Ok(NewDevice {
            link: link.clone(),
            collect_token,
            keys,
            device_name,
            portable,
        })
    }

    /// The code to show, when this device is the one showing it.
    pub fn link(&self) -> &ApprovalLink {
        &self.link
    }

    /// Ask once whether the existing device has approved. Anything that
    /// doesn't check out is refused, and the approval is used up either way.
    pub async fn collect(&self, store: &KeyStore) -> Result<Option<SignedIn>, ApproveError> {
        let reply: Result<ApprovalCollected, TlsError> = self
            .link
            .server()
            .post(
                "/api/v1/approvals/collect",
                &ApprovalCollect {
                    approval_id: self.link.id.clone(),
                    collect_token: self.collect_token.clone(),
                },
            )
            .await;
        let Some(granted) = reply.map_err(expired_or)?.granted else {
            return Ok(None);
        };
        let plain = self
            .link
            .key
            .open(&self.link.id, "grant", &granted.grant)
            .map_err(|_| {
                ApproveError::Invalid("the answer is not from the device that was shown the code")
            })?;
        let grant: ApprovalGrantPlain = serde_json::from_slice(&plain)
            .map_err(|_| ApproveError::Invalid("the answer is damaged"))?;
        let account = Keys::from_bytes(&grant.keys)?;
        // The identity key we were just given has to be the one that signed
        // exactly the keys this device made.
        verify_device(&account.identity_public(), &granted.device)?;
        let ours = self.keys.signing.verifying_key().to_bytes();
        if granted.device.signing_public != ours
            || granted.device.sealing_public != self.keys.sealing_public()
        {
            return Err(ApproveError::Invalid("the signed device is not this one"));
        }

        let account_id = KeyId {
            server_fingerprint: self.link.fingerprint.clone(),
            username: grant.username.clone(),
            kind: KeyKind::Account,
        };
        let device_id = KeyId {
            kind: KeyKind::Device,
            ..account_id.clone()
        };
        store.put(&account_id, &account.to_bytes()).await?;
        store.put(&device_id, &self.keys.to_bytes()).await?;
        Ok(Some(SignedIn {
            profile: Profile {
                server_address: self.link.address.clone(),
                server_fingerprint: self.link.fingerprint.clone(),
                server_name: self.link.server_name.clone(),
                username: grant.username,
                display_name: grant.display_name,
                admin: granted.joined.admin,
                account_id: granted.joined.account_id,
                device_id: granted.joined.device_id,
                device_name: self.device_name.clone(),
                portable: self.portable,
            },
        }))
    }

    /// Wait for the existing device to approve this one, up to `timeout`,
    /// keep the keys it sends and return the profile, as after a password
    /// sign-in.
    pub async fn wait(
        &self,
        store: &KeyStore,
        timeout: Duration,
    ) -> Result<SignedIn, ApproveError> {
        let deadline = tokio::time::Instant::now() + timeout;
        loop {
            if let Some(signed_in) = self.collect(store).await? {
                return Ok(signed_in);
            }
            if tokio::time::Instant::now() + POLL > deadline {
                return Err(ApproveError::TimedOut);
            }
            tokio::time::sleep(POLL).await;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const FP: &str = "00:11:22:33:44:55:66:77:88:99:AA:BB:CC:DD:EE:FF:00:11:22:33:44:55:66:77:88:99:AA:BB:CC:DD:EE:FF";

    fn link() -> ApprovalLink {
        ApprovalLink {
            address: "home.lan:8443".into(),
            server_name: "Chris & Sam's home #1".into(),
            fingerprint: FP.into(),
            id: "0123456789abcdef0123456789abcdef".into(),
            key: ApprovalKey::generate(),
        }
    }

    #[test]
    fn a_link_survives_its_text() {
        let l = link();
        let text = l.link();
        assert!(text.starts_with("hab-bot://approve?"));
        assert!(!text.contains(' ') && !text.contains('#'));
        assert_eq!(ApprovalLink::parse(&text).unwrap(), l);
        assert_eq!(ApprovalLink::parse(&format!(" {text}\n")).unwrap(), l);
        assert!(l.qr_svg().contains("<svg"));
    }

    #[test]
    fn damaged_links_are_refused() {
        let good = link().link();
        for bad in [
            "".to_string(),
            "hab-bot://approve".into(),
            "hab-bot://sign-in?address=home.lan".into(),
            good.replace("&key=", "&k="),
            good.replace("id=0123", "id=01"),
            good.replace("fingerprint=", "fingerprint=AA"),
            format!("{good}&bad"),
            good[..good.len() - 4].to_string(),
        ] {
            assert!(ApprovalLink::parse(&bad).is_err(), "{bad}");
        }
    }
}
