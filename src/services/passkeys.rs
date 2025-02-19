use constant_time_eq::constant_time_eq;
use p256::ecdsa::signature::Verifier;
use p256::ecdsa::{Signature, VerifyingKey};
use p256::pkcs8::DecodePublicKey;
use rand::{thread_rng, Rng};
use serde::{Deserialize, Serialize};
use serde_with::base64::{Base64, UrlSafe};
use serde_with::formats::Unpadded;
use serde_with::{serde_as, PickFirst};
use sha2::{Digest, Sha256};
use sqlx::SqlitePool;
use url::Url;

#[derive(Debug, Clone)]
pub struct PasskeyService {
    db: SqlitePool,
    rp_id: String,
    origin: Url,
}

impl PasskeyService {
    pub fn new(db: SqlitePool, base_url: &Url) -> PasskeyService {
        let rp_id = base_url.host_str().unwrap().into();
        PasskeyService { db, rp_id, origin: base_url.clone() }
    }

    pub async fn any_registered(&self) -> Result<bool, sqlx::Error> {
        sqlx::query!(r"select count(passkey_id) as n from passkey")
            .fetch_one(&self.db)
            .await
            .map(|r| r.n > 0)
    }

    pub async fn start_registration(
        &self,
        username: &str,
        user_id: &[u8],
    ) -> Result<RegistrationChallenge, sqlx::Error> {
        Ok(RegistrationChallenge {
            rp_id: self.rp_id.clone(),
            username: username.into(),
            user_id: user_id.into(),
            passkey_ids: self.passkey_ids().await?,
        })
    }

    pub async fn finish_registration(
        &self,
        resp: RegistrationResponse,
    ) -> Result<(), anyhow::Error> {
        // Try decoding the P-256 public key from its DER encoding.
        VerifyingKey::from_public_key_der(&resp.public_key)?;

        // Decode and validate the client data.
        if !CollectedClientData::is_valid(
            &resp.client_data_json,
            &self.origin,
            "webauthn.create",
            None,
        ) {
            anyhow::bail!("invalid client data");
        }

        // Decode and validate the authenticator data.
        let passkey_id = parse_authenticator_data(&resp.authenticator_data, &self.rp_id)?
            .ok_or_else(|| anyhow::anyhow!("missing passkey id"))?;

        // Insert the passkey ID and DER-encoded public key into the database.
        sqlx::query!(
            r"insert into passkey (passkey_id, public_key_spki) values (?, ?)",
            passkey_id,
            resp.public_key,
        )
        .execute(&self.db)
        .await?;

        Ok(())
    }

    pub async fn start_authentication(&self) -> Result<AuthenticationChallenge, sqlx::Error> {
        // Find all passkey IDs.
        let passkey_ids = self.passkey_ids().await?;

        // Generate a random challenge.
        let challenge = thread_rng().gen::<[u8; 32]>();

        Ok(AuthenticationChallenge { rp_id: self.rp_id.clone(), challenge, passkey_ids })
    }

    pub async fn finish_authentication(
        &self,
        resp: AuthenticationResponse,
        challenge: [u8; 32],
    ) -> Result<bool, sqlx::Error> {
        // Validate the collected client data.
        if !CollectedClientData::is_valid(
            &resp.client_data_json,
            &self.origin,
            "webauthn.get",
            Some(&challenge),
        ) {
            tracing::warn!(cdj=?resp.client_data_json, "invalid collected client data");
            return Ok(false);
        }

        // Decode and validate the authenticator data.
        if parse_authenticator_data(&resp.authenticator_data, &self.rp_id).is_err() {
            tracing::warn!(ad=?resp.authenticator_data, "invalid authenticator data");
            return Ok(false);
        }

        // Find the passkey by ID.
        let public_key_spki =
            sqlx::query!(r"select public_key_spki from passkey where passkey_id = ?", resp.raw_id)
                .fetch_optional(&self.db)
                .await?
                .map(|r| r.public_key_spki);
        let Some(public_key_spki) = public_key_spki else {
            tracing::warn!(passkey_id=?resp.raw_id, "unable to find passkey");
            return Ok(false);
        };

        // Decode the public key.
        let Ok(public_key) = VerifyingKey::from_public_key_der(&public_key_spki) else {
            tracing::warn!(passkey_id=?resp.raw_id, "unable to decode public key");
            return Ok(false);
        };

        // Re-calculate the signed material.
        let mut signed = resp.authenticator_data.clone();
        let cdj_hash = Sha256::new().chain_update(&resp.client_data_json).finalize();
        signed.extend(cdj_hash);

        // Decode the signature.
        let Ok(signature) = Signature::from_der(resp.signature.as_slice()) else {
            tracing::warn!(passkey_id=?resp.raw_id, "unable to decode signature");
            return Ok(false);
        };

        // Verify the signature.
        Ok(public_key.verify(&signed, &signature).is_ok())
    }

    async fn passkey_ids(&self) -> Result<Vec<Vec<u8>>, sqlx::Error> {
        Ok(sqlx::query!(r"select passkey_id from passkey")
            .fetch_all(&self.db)
            .await?
            .into_iter()
            .map(|r| r.passkey_id)
            .collect::<Vec<Vec<u8>>>())
    }
}

#[serde_as]
#[derive(Debug, Serialize, Deserialize)]
pub struct RegistrationChallenge {
    #[serde(rename = "rpId")]
    pub rp_id: String,

    #[serde(rename = "userIdBase64")]
    #[serde_as(as = "PickFirst<(Base64, Base64<UrlSafe, Unpadded>)>")]
    pub user_id: Vec<u8>,

    pub username: String,

    #[serde(rename = "passkeyIdsBase64")]
    #[serde_as(as = "Vec<PickFirst<(Base64, Base64<UrlSafe, Unpadded>)>>")]
    pub passkey_ids: Vec<Vec<u8>>,
}

#[serde_as]
#[derive(Debug, Serialize, Deserialize)]
pub struct RegistrationResponse {
    #[serde(rename = "clientDataJSONBase64")]
    #[serde_as(as = "PickFirst<(Base64, Base64<UrlSafe, Unpadded>)>")]
    pub client_data_json: Vec<u8>,

    #[serde(rename = "authenticatorDataBase64")]
    #[serde_as(as = "PickFirst<(Base64, Base64<UrlSafe, Unpadded>)>")]
    pub authenticator_data: Vec<u8>,

    #[serde(rename = "publicKeyBase64")]
    #[serde_as(as = "PickFirst<(Base64, Base64<UrlSafe, Unpadded>)>")]
    pub public_key: Vec<u8>,
}
#[serde_as]
#[derive(Debug, Serialize, Deserialize)]
pub struct AuthenticationChallenge {
    #[serde(rename = "rpId")]
    pub rp_id: String,

    #[serde(rename = "challengeBase64")]
    #[serde_as(as = "Base64")]
    pub challenge: [u8; 32],

    #[serde(rename = "passkeyIdsBase64")]
    #[serde_as(as = "Vec<PickFirst<(Base64, Base64<UrlSafe, Unpadded>)>>")]
    pub passkey_ids: Vec<Vec<u8>>,
}

#[serde_as]
#[derive(Debug, Serialize, Deserialize)]
pub struct AuthenticationResponse {
    #[serde(rename = "rawIdBase64")]
    #[serde_as(as = "PickFirst<(Base64, Base64<UrlSafe, Unpadded>)>")]
    pub raw_id: Vec<u8>,

    #[serde(rename = "clientDataJSONBase64")]
    #[serde_as(as = "PickFirst<(Base64, Base64<UrlSafe, Unpadded>)>")]
    pub client_data_json: Vec<u8>,

    #[serde(rename = "authenticatorDataBase64")]
    #[serde_as(as = "PickFirst<(Base64, Base64<UrlSafe, Unpadded>)>")]
    pub authenticator_data: Vec<u8>,

    #[serde(rename = "signatureBase64")]
    #[serde_as(as = "PickFirst<(Base64, Base64<UrlSafe, Unpadded>)>")]
    pub signature: Vec<u8>,
}

#[serde_as]
#[derive(Debug, Deserialize)]
struct CollectedClientData {
    #[serde_as(as = "Option<PickFirst<(Base64, Base64<UrlSafe, Unpadded>)>>")]
    challenge: Option<Vec<u8>>,

    origin: Url,

    #[serde(rename = "type")]
    type_: String,

    #[serde(rename = "crossOrigin")]
    cross_origin: Option<bool>,
}

impl CollectedClientData {
    fn is_valid(json: &[u8], origin: &Url, action: &str, challenge: Option<&[u8]>) -> bool {
        let Ok(cdj) = serde_json::from_slice::<CollectedClientData>(json) else {
            return false;
        };
        cdj.type_ == action
            && !cdj.cross_origin.unwrap_or(false)
            && &cdj.origin == origin
            && challenge
                .map(|v| constant_time_eq(&cdj.challenge.unwrap_or_default(), v))
                .unwrap_or(true)
    }
}

fn parse_authenticator_data(ad: &[u8], rp_id: &str) -> Result<Option<Vec<u8>>, anyhow::Error> {
    let rp_hash = Sha256::new().chain_update(rp_id.as_bytes()).finalize();
    anyhow::ensure!(constant_time_eq(&rp_hash, &ad[..32]), "invalid RP ID hash");
    anyhow::ensure!(ad[32] & 1 != 0, "user presence flag not set");
    if ad.len() > 55 {
        let cred_id_len = u16::from_be_bytes(ad[53..55].try_into().unwrap()) as usize;
        anyhow::ensure!(ad.len() >= 55 + cred_id_len, "bad credential ID size");
        Ok(Some(ad[55..55 + cred_id_len].to_vec()))
    } else {
        Ok(None)
    }
}
