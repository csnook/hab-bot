//! Signing in on another device with the username and password.
//!
//! An OPAQUE login proves the password without sending it, and only then does
//! the server release the key bundle. The bundle opens with a key that only
//! the password (through OPAQUE's export key) can make. The device then makes
//! its own keys, has the identity key sign them, and asks the server to add it
//! (ADR 0004).

use crate::keystore::{KeyId, KeyKind, KeyStore, KeyStoreError};
use crate::profile::Profile;
use crate::tls::{Pinned, TlsError};
use hab_proto::opaque_ke::{
    ClientLogin, ClientLoginFinishParameters, CredentialResponse, Identifiers,
};
use hab_proto::wire::{
    valid_username, Joined, Kdf, LoginDevice, LoginFinish, LoginFinished, LoginStart, LoginStarted,
};
use hab_proto::{argon2_with, DeviceKeys, KeyError, Keys, Suite, ARGON_MEMORY_KIB};

#[derive(Debug, thiserror::Error)]
pub enum SignInError {
    #[error("{0}")]
    Input(&'static str),
    #[error("The username or password is wrong.")]
    WrongLogin,
    #[error(transparent)]
    Server(#[from] TlsError),
    #[error("signing in failed: {0}")]
    Opaque(String),
    #[error("the account's keys could not be opened: {0}")]
    Keys(#[from] KeyError),
    #[error(transparent)]
    Store(#[from] KeyStoreError),
}

pub struct SignInRequest {
    pub address: String,
    /// The fingerprint to pin: from the sign-in code, or confirmed by the user.
    pub fingerprint: String,
    pub server_name: String,
    pub username: String,
    pub password: String,
    pub device_name: String,
    pub portable: bool,
}

#[derive(Debug)]
pub struct SignedIn {
    pub profile: Profile,
}

/// The cost a server may ask of the password hardening. Anything outside is
/// refused rather than run.
fn kdf_in_bounds(k: &Kdf) -> bool {
    k.alg == "argon2id"
        && (8..=4 * ARGON_MEMORY_KIB).contains(&k.memory_kib)
        && (1..=10).contains(&k.passes)
        && (1..=16).contains(&k.lanes)
}

/// Sign in, make this device's keys, and have the server add the device.
///
/// The keys go in `store` before the server is asked to add the device, as
/// when joining, and are removed again if it refuses.
pub async fn sign_in(req: SignInRequest, store: &KeyStore) -> Result<SignedIn, SignInError> {
    if !valid_username(&req.username) || req.password.is_empty() {
        return Err(SignInError::Input("Enter your username and password."));
    }
    if req.device_name.trim().is_empty() {
        return Err(SignInError::Input("This device needs a name."));
    }
    let server = Pinned::new(&req.address, &req.fingerprint);

    let mut rng = hab_proto::opaque_ke::rand::rngs::OsRng;
    let started = ClientLogin::<Suite>::start(&mut rng, req.password.as_bytes())
        .map_err(|e| SignInError::Opaque(e.to_string()))?;
    let reply: LoginStarted = server
        .post(
            "/api/v1/login/start",
            &LoginStart {
                username: req.username.clone(),
                credential_request: started.message.serialize().to_vec(),
            },
        )
        .await?;
    if !kdf_in_bounds(&reply.kdf) {
        return Err(SignInError::Opaque(
            "the server asked for password hardening this app won't do".into(),
        ));
    }
    let response = CredentialResponse::<Suite>::deserialize(&reply.credential_response)
        .map_err(|e| SignInError::Opaque(e.to_string()))?;

    // Argon2id at 64 MiB takes a moment, so keep it off the async threads.
    let (password, state, kdf) = (req.password.clone(), started.state, reply.kdf.clone());
    let finished = tokio::task::spawn_blocking(move || {
        let ksf = argon2_with(kdf.memory_kib, kdf.passes, kdf.lanes);
        let mut rng = hab_proto::opaque_ke::rand::rngs::OsRng;
        state.finish(
            &mut rng,
            password.as_bytes(),
            response,
            ClientLoginFinishParameters::new(None, Identifiers::default(), Some(&ksf)),
        )
    })
    .await
    .map_err(|e| SignInError::Opaque(e.to_string()))?
    // A wrong password, or a username the server made up an answer for.
    .map_err(|_| SignInError::WrongLogin)?;

    let released: LoginFinished = match server
        .post(
            "/api/v1/login/finish",
            &LoginFinish {
                login_id: reply.login_id.clone(),
                credential_finalization: finished.message.serialize().to_vec(),
            },
        )
        .await
    {
        Ok(r) => r,
        Err(TlsError::Status { status: 401, .. }) => return Err(SignInError::WrongLogin),
        Err(e) => return Err(e.into()),
    };

    let keys = Keys::open(
        &released.bundle,
        finished.export_key.as_slice(),
        &req.username,
    )?;
    if keys.identity_public().as_slice() != released.identity_public.as_slice() {
        return Err(SignInError::Opaque(
            "the server's identity key is not the one in the bundle".into(),
        ));
    }
    let device_keys = DeviceKeys::generate();
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

    let added: Result<Joined, TlsError> = server
        .post(
            "/api/v1/login/device",
            &LoginDevice {
                login_id: reply.login_id,
                device,
            },
        )
        .await;
    let joined = match added {
        Ok(j) => j,
        Err(e) => {
            // The device wasn't added, so its keys aren't needed.
            let _ = store.delete(&account_id).await;
            let _ = store.delete(&device_id).await;
            return Err(e.into());
        }
    };

    Ok(SignedIn {
        profile: Profile {
            server_address: req.address,
            server_fingerprint: req.fingerprint,
            server_name: req.server_name,
            username: req.username,
            display_name: released.display_name,
            admin: joined.admin,
            account_id: joined.account_id,
            device_id: joined.device_id,
            device_name: req.device_name.trim().to_string(),
            portable: req.portable,
        },
    })
}
