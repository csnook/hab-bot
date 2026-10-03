//! Creating the first account with the setup code.

use crate::keystore::{KeyId, KeyKind, KeyStore, KeyStoreError};
use crate::profile::Profile;
use crate::strength::check_password;
use crate::tls::{Pinned, TlsError};
use hab_proto::opaque_ke::{
    ClientRegistration, ClientRegistrationFinishParameters, Identifiers, RegistrationResponse,
};
use hab_proto::wire::{self, Algs, JoinFinish, JoinStart, JoinStarted, Joined as JoinedReply, Kdf};
use hab_proto::{argon2, DeviceKeys, Keys, Suite, ARGON_LANES, ARGON_MEMORY_KIB, ARGON_PASSES};

#[derive(Debug, thiserror::Error)]
pub enum JoinError {
    #[error("{0}")]
    Input(&'static str),
    #[error(transparent)]
    Server(#[from] TlsError),
    #[error("the password could not be registered: {0}")]
    Opaque(String),
    #[error(transparent)]
    Keys(#[from] KeyStoreError),
}

pub struct JoinRequest {
    pub address: String,
    /// The fingerprint the user confirmed.
    pub fingerprint: String,
    pub server_name: String,
    pub setup_code: String,
    pub username: String,
    pub display_name: String,
    pub password: String,
    pub device_name: String,
    pub portable: bool,
}

pub struct Joined {
    pub profile: Profile,
}

/// Register with OPAQUE, make the keys, and create the account.
///
/// The password never leaves this function except as OPAQUE's messages. The
/// keys are encrypted with OPAQUE's export key into a bundle for the server,
/// and the device's own copies go in `store` before the account is created,
/// so a crash can't leave an account whose device keys were never kept.
pub async fn join(req: JoinRequest, store: &KeyStore) -> Result<Joined, JoinError> {
    if !wire::valid_username(&req.username) {
        return Err(JoinError::Input(
            "Usernames are lower-case letters, digits, dots, dashes and underscores.",
        ));
    }
    if !wire::valid_display_name(&req.display_name) {
        return Err(JoinError::Input("Enter a display name."));
    }
    if !check_password(&req.password, &[&req.username, &req.display_name]).ok {
        return Err(JoinError::Input("That password is too easy to guess."));
    }
    let server = Pinned::new(&req.address, &req.fingerprint);

    let mut rng = hab_proto::opaque_ke::rand::rngs::OsRng;
    let started = ClientRegistration::<Suite>::start(&mut rng, req.password.as_bytes())
        .map_err(|e| JoinError::Opaque(e.to_string()))?;
    let reply: JoinStarted = server
        .post(
            "/api/v1/join/start",
            &JoinStart {
                setup_code: req.setup_code.clone(),
                username: req.username.clone(),
                registration_request: started.message.serialize().to_vec(),
            },
        )
        .await?;
    let response = RegistrationResponse::<Suite>::deserialize(&reply.registration_response)
        .map_err(|e| JoinError::Opaque(e.to_string()))?;

    // Argon2id at 64 MiB takes a moment, so keep it off the async threads.
    let password = req.password.clone();
    let state = started.state;
    let finished = tokio::task::spawn_blocking(move || {
        let ksf = argon2();
        let mut rng = hab_proto::opaque_ke::rand::rngs::OsRng;
        state.finish(
            &mut rng,
            password.as_bytes(),
            response,
            ClientRegistrationFinishParameters::new(Identifiers::default(), Some(&ksf)),
        )
    })
    .await
    .map_err(|e| JoinError::Opaque(e.to_string()))?
    .map_err(|e| JoinError::Opaque(e.to_string()))?;

    let keys = Keys::generate();
    let device_keys = DeviceKeys::generate();
    let bundle = keys.seal(finished.export_key.as_slice(), &req.username);
    let device = device_keys.record(&keys.identity, req.portable);

    let account_id = KeyId {
        server_fingerprint: req.fingerprint.clone(),
        username: req.username.clone(),
        kind: KeyKind::Account,
    };
    let device_id = KeyId {
        kind: KeyKind::Device,
        ..account_id.clone()
    };
    store.put(&account_id, &keys.to_bytes()).await?;
    store.put(&device_id, &device_keys.to_bytes()).await?;

    let sent: Result<JoinedReply, TlsError> = server
        .post(
            "/api/v1/join/finish",
            &JoinFinish {
                setup_code: req.setup_code,
                username: req.username.clone(),
                display_name: req.display_name.clone(),
                registration_upload: finished.message.serialize().to_vec(),
                identity_alg: Algs::IDENTITY.into(),
                identity_public: keys.identity_public().to_vec(),
                kdf: Kdf {
                    alg: "argon2id".into(),
                    memory_kib: ARGON_MEMORY_KIB,
                    passes: ARGON_PASSES,
                    lanes: ARGON_LANES,
                },
                bundle,
                device,
            },
        )
        .await;
    let joined = match sent {
        Ok(j) => j,
        Err(e) => {
            // Nothing was created, so the keys made for it are not needed.
            let _ = store.delete(&account_id).await;
            let _ = store.delete(&device_id).await;
            return Err(e.into());
        }
    };

    Ok(Joined {
        profile: Profile {
            server_address: req.address,
            server_fingerprint: req.fingerprint,
            server_name: req.server_name,
            username: req.username,
            display_name: req.display_name,
            admin: joined.admin,
            account_id: joined.account_id,
            device_id: joined.device_id,
            device_name: req.device_name,
            portable: req.portable,
        },
    })
}
