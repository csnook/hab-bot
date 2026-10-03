//! The sign-in code: a link, shown as a QR code too, that carries a server's
//! address, name and certificate fingerprint. Any signed-in device can show it
//! (Settings → Account), so a new device pins the right certificate without
//! the user comparing fingerprints by eye.
//!
//! It holds nothing secret: signing in still takes the username and password.

use crate::profile::Profile;
use crate::tls::parse_address;

const SCHEME: &str = "hab-bot://sign-in";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SignInCode {
    pub address: String,
    pub server_name: String,
    /// `AA:BB:..`, SHA-256 of the server's certificate.
    pub fingerprint: String,
}

/// What the user typed into the sign-in screen's first field.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SignInTarget {
    /// A sign-in code: the fingerprint is already known.
    Code(SignInCode),
    /// Just an address: the fingerprint has to be shown and confirmed.
    Address(String),
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum CodeError {
    #[error("That isn't a sign-in code or a server address.")]
    Invalid,
}

impl SignInCode {
    pub fn for_profile(p: &Profile) -> SignInCode {
        SignInCode {
            address: p.server_address.clone(),
            server_name: p.server_name.clone(),
            fingerprint: p.server_fingerprint.clone(),
        }
    }

    /// The link, for copying or opening on another device.
    pub fn link(&self) -> String {
        format!(
            "{SCHEME}?address={}&name={}&fingerprint={}",
            encode(&self.address),
            encode(&self.server_name),
            encode(&self.fingerprint)
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
}

/// Read a sign-in code (its link), or else a server address.
pub fn parse_target(text: &str) -> Result<SignInTarget, CodeError> {
    let text = text.trim();
    if let Some(query) = text
        .strip_prefix(SCHEME)
        .and_then(|rest| rest.strip_prefix('?'))
    {
        let (mut address, mut name, mut fingerprint) = (None, None, None);
        for pair in query.split('&') {
            let (key, value) = pair.split_once('=').ok_or(CodeError::Invalid)?;
            let slot = match key {
                "address" => &mut address,
                "name" => &mut name,
                "fingerprint" => &mut fingerprint,
                _ => continue,
            };
            *slot = Some(decode(value).ok_or(CodeError::Invalid)?);
        }
        let (Some(address), Some(server_name), Some(fingerprint)) = (address, name, fingerprint)
        else {
            return Err(CodeError::Invalid);
        };
        if parse_address(&address).is_err() || !plausible_fingerprint(&fingerprint) {
            return Err(CodeError::Invalid);
        }
        return Ok(SignInTarget::Code(SignInCode {
            address,
            server_name,
            fingerprint,
        }));
    }
    if text.contains("://") && !text.starts_with("https://") {
        return Err(CodeError::Invalid);
    }
    parse_address(text).map_err(|_| CodeError::Invalid)?;
    Ok(SignInTarget::Address(text.to_string()))
}

fn plausible_fingerprint(f: &str) -> bool {
    let digits: String = f.chars().filter(|c| *c != ':').collect();
    digits.len() == 64 && digits.chars().all(|c| c.is_ascii_hexdigit())
}

fn encode(s: &str) -> String {
    let mut out = String::new();
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b'~') {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

fn decode(s: &str) -> Option<String> {
    let bytes = s.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' {
            let hex = s.get(i + 1..i + 3)?;
            out.push(u8::from_str_radix(hex, 16).ok()?);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8(out).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    const FP: &str = "00:11:22:33:44:55:66:77:88:99:AA:BB:CC:DD:EE:FF:00:11:22:33:44:55:66:77:88:99:AA:BB:CC:DD:EE:FF";

    fn code() -> SignInCode {
        SignInCode {
            address: "home.lan:8443".into(),
            server_name: "Chris & Sam's home #1".into(),
            fingerprint: FP.into(),
        }
    }

    #[test]
    fn a_code_survives_its_link() {
        let link = code().link();
        assert!(link.starts_with("hab-bot://sign-in?"));
        assert!(!link.contains(' ') && !link.contains('#'));
        assert_eq!(parse_target(&link), Ok(SignInTarget::Code(code())));
        assert_eq!(
            parse_target(&format!("  {link}\n")),
            Ok(SignInTarget::Code(code()))
        );
    }

    #[test]
    fn the_qr_code_carries_the_link() {
        let svg = code().qr_svg();
        assert!(svg.starts_with("<?xml") || svg.contains("<svg"));
    }

    #[test]
    fn an_address_is_accepted_without_a_fingerprint() {
        assert_eq!(
            parse_target("home.lan:8443"),
            Ok(SignInTarget::Address("home.lan:8443".into()))
        );
    }

    #[test]
    fn damaged_codes_are_refused() {
        for bad in [
            "",
            "hab-bot://sign-in",
            "hab-bot://sign-in?address=home.lan",
            "hab-bot://sign-in?address=home.lan&name=x&fingerprint=AA:BB",
            "hab-bot://sign-in?address=a%20b&name=x&fingerprint=",
            "hab-bot://sign-in?address=home.lan&name=%ZZ&fingerprint=",
            "ftp://home.lan",
            "two words",
        ] {
            assert_eq!(parse_target(bad), Err(CodeError::Invalid), "{bad}");
        }
    }
}
