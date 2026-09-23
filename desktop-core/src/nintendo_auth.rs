//! Nintendo Account browser-login boundary.
//!
//! The desktop shell opens the returned URL in the system browser. The user
//! copies the Nintendo "Select this account" link and pastes it back into the
//! app; `finish` validates that link. Passwords never pass through the widget
//! UI.

use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
use serde::Deserialize;
use sha2::{Digest, Sha256};
use url::{form_urlencoded, Url};

pub const NINTENDO_CLIENT_ID: &str = "71b963c1b7b6d119";
pub const REDIRECT_URI: &str = "npf71b963c1b7b6d119://auth";
const AUTHORIZE_URL: &str = "https://accounts.nintendo.com/connect/1.0.0/authorize";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoginAttempt {
    pub state: String,
    verifier: String,
    pub authorize_url: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoginCallback {
    pub session_token_code: String,
    pub verifier: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NintendoIdentity {
    pub account_id: String,
    pub display_name: String,
    pub language: Option<String>,
    pub country: Option<String>,
    pub birthday: String,
    pub id_token: String,
    pub session_token: String,
}

#[derive(Debug, Deserialize)]
struct SessionTokenResponse {
    session_token: String,
}

#[derive(Debug, Deserialize)]
struct NintendoTokenResponse {
    access_token: String,
    id_token: String,
}

#[derive(Debug, Deserialize)]
struct UserProfileResponse {
    id: serde_json::Value,
    nickname: Option<String>,
    language: Option<String>,
    country: Option<String>,
    birthday: Option<String>,
}

pub fn begin_login() -> Result<LoginAttempt, String> {
    let state = random_url_token(32)?;
    let verifier = random_url_token(32)?;
    let challenge = URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()));

    let authorize_url = form_urlencoded::Serializer::new(format!("{AUTHORIZE_URL}?"))
        .append_pair("state", &state)
        .append_pair("redirect_uri", REDIRECT_URI)
        .append_pair("client_id", NINTENDO_CLIENT_ID)
        .append_pair(
            "scope",
            "openid user user.birthday user.mii user.screenName",
        )
        .append_pair("response_type", "session_token_code")
        .append_pair("session_token_code_challenge", &challenge)
        .append_pair("session_token_code_challenge_method", "S256")
        .append_pair("theme", "login_form")
        .finish();

    Ok(LoginAttempt {
        state,
        verifier,
        authorize_url,
    })
}

impl LoginAttempt {
    /// Validate the copied Nintendo account-selection link and return the
    /// one-time exchange inputs. The caller must discard this attempt after
    /// calling this method.
    pub fn finish(self, callback_url: &str) -> Result<LoginCallback, String> {
        let callback = Url::parse(callback_url).map_err(|error| error.to_string())?;
        let is_nintendo_redirect =
            callback.scheme() == "npf71b963c1b7b6d119" && callback.host_str() == Some("auth");
        let is_copied_nintendo_link = callback.scheme() == "https"
            && callback.host_str() == Some("accounts.nintendo.com")
            && callback.path() == "/connect/1.0.0/authorize";
        if !is_nintendo_redirect && !is_copied_nintendo_link {
            return Err("请粘贴 Nintendo“选择此人”按钮的链接".to_owned());
        }

        let query = callback_parameters(&callback);
        let returned_state = query
            .get("st")
            .or_else(|| query.get("state"))
            .ok_or_else(|| "Nintendo selection link is missing state".to_owned())?;
        if returned_state != &self.state {
            return Err("Nintendo login state did not match".to_owned());
        }

        let code = query
            .get("de")
            .or_else(|| query.get("session_token_code"))
            .ok_or_else(|| "Nintendo selection link is missing session token code".to_owned())?;
        if code.is_empty() {
            return Err("Nintendo selection link contains an empty session token code".to_owned());
        }

        Ok(LoginCallback {
            session_token_code: code.clone(),
            verifier: self.verifier,
        })
    }
}

/// Exchanges the short-lived browser callback for a Nintendo session token
/// and the minimal account profile needed by the account registry.
///
/// This intentionally stops before SplatNet's game-web-token exchange. That
/// exchange belongs to the SplatNet client and can refresh independently of
/// the account-login layer.
pub async fn exchange_session_token(callback: LoginCallback) -> Result<NintendoIdentity, String> {
    let client = reqwest::Client::new();
    let session_response = client
        .post("https://accounts.nintendo.com/connect/1.0.0/api/session_token")
        .header("User-Agent", "OnlineLounge/1.0 NASDKAPI")
        .form(&[
            ("client_id", NINTENDO_CLIENT_ID),
            ("session_token_code", callback.session_token_code.as_str()),
            ("session_token_code_verifier", callback.verifier.as_str()),
        ])
        .send()
        .await
        .map_err(|error| format!("Nintendo session request failed: {error}"))?
        .error_for_status()
        .map_err(|error| format!("Nintendo session request failed: {error}"))?
        .json::<SessionTokenResponse>()
        .await
        .map_err(|error| format!("Nintendo session response was invalid: {error}"))?;

    identity_from_session_token(&client, &session_response.session_token).await
}

/// Rehydrates the Nintendo identity and short-lived `id_token` from a saved
/// session token. This lets SplatNet credentials be refreshed without asking
/// the user to log in again.
pub async fn refresh_identity(session_token: &str) -> Result<NintendoIdentity, String> {
    let client = reqwest::Client::new();
    identity_from_session_token(&client, session_token).await
}

async fn identity_from_session_token(
    client: &reqwest::Client,
    session_token: &str,
) -> Result<NintendoIdentity, String> {
    let token_response = client
        .post("https://accounts.nintendo.com/connect/1.0.0/api/token")
        .json(&serde_json::json!({
            "client_id": NINTENDO_CLIENT_ID,
            "session_token": session_token,
            "grant_type": "urn:ietf:params:oauth:grant-type:jwt-bearer-session-token"
        }))
        .send()
        .await
        .map_err(|error| format!("Nintendo identity request failed: {error}"))?
        .error_for_status()
        .map_err(|error| format!("Nintendo identity request failed: {error}"))?
        .json::<NintendoTokenResponse>()
        .await
        .map_err(|error| format!("Nintendo identity response was invalid: {error}"))?;

    let NintendoTokenResponse {
        access_token,
        id_token,
    } = token_response;

    let profile = client
        .get("https://api.accounts.nintendo.com/2.0.0/users/me")
        .bearer_auth(access_token)
        .send()
        .await
        .map_err(|error| format!("Nintendo profile request failed: {error}"))?
        .error_for_status()
        .map_err(|error| format!("Nintendo profile request failed: {error}"))?
        .json::<UserProfileResponse>()
        .await
        .map_err(|error| format!("Nintendo profile response was invalid: {error}"))?;

    let account_id = match profile.id {
        serde_json::Value::String(value) => value,
        value => value.to_string(),
    };
    if account_id.is_empty() || account_id == "null" {
        return Err("Nintendo profile did not include an account id".to_owned());
    }

    let birthday = profile
        .birthday
        .ok_or_else(|| "Nintendo profile did not include a birthday".to_owned())?;

    Ok(NintendoIdentity {
        account_id,
        display_name: profile
            .nickname
            .filter(|value| !value.trim().is_empty())
            .unwrap_or_else(|| "Nintendo Account".to_owned()),
        language: profile.language,
        country: profile.country,
        birthday,
        id_token,
        session_token: session_token.to_owned(),
    })
}

fn random_url_token(length: usize) -> Result<String, String> {
    let mut bytes = vec![0_u8; length];
    getrandom::fill(&mut bytes).map_err(|error| error.to_string())?;
    Ok(URL_SAFE_NO_PAD.encode(bytes))
}

fn callback_parameters(url: &Url) -> std::collections::HashMap<String, String> {
    let mut parameters: std::collections::HashMap<_, _> = url.query_pairs().into_owned().collect();
    if let Some(fragment) = url.fragment() {
        for (key, value) in form_urlencoded::parse(fragment.trim_start_matches('?').as_bytes()) {
            parameters
                .entry(key.into_owned())
                .or_insert(value.into_owned());
        }
    }
    parameters
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn creates_pkce_authorization_request() {
        let attempt = begin_login().unwrap();
        let url = Url::parse(&attempt.authorize_url).unwrap();
        let params: std::collections::HashMap<_, _> = url.query_pairs().into_owned().collect();

        assert_eq!(
            url.origin().ascii_serialization(),
            "https://accounts.nintendo.com"
        );
        assert_eq!(
            params.get("client_id"),
            Some(&NINTENDO_CLIENT_ID.to_owned())
        );
        assert_eq!(params.get("redirect_uri"), Some(&REDIRECT_URI.to_owned()));
        assert_eq!(
            params.get("response_type"),
            Some(&"session_token_code".to_owned())
        );
        assert_eq!(
            params.get("session_token_code_challenge_method"),
            Some(&"S256".to_owned())
        );
        assert_eq!(params.get("state"), Some(&attempt.state));
    }

    #[test]
    fn accepts_matching_callback_and_returns_verifier() {
        let attempt = begin_login().unwrap();
        let callback = format!("{REDIRECT_URI}?de=session-code&st={}", attempt.state);

        let result = attempt.clone().finish(&callback).unwrap();
        assert_eq!(result.session_token_code, "session-code");
        assert!(!result.verifier.is_empty());
    }

    #[test]
    fn rejects_state_mismatch() {
        let attempt = begin_login().unwrap();
        let result = attempt
            .finish("https://accounts.nintendo.com/connect/1.0.0/authorize?de=code&st=wrong");
        assert!(result.unwrap_err().contains("state did not match"));
    }

    #[test]
    fn accepts_the_copied_nintendo_selection_link() {
        let attempt = begin_login().unwrap();
        let copied_link = format!(
            "https://accounts.nintendo.com/connect/1.0.0/authorize?de=session-code&st={}",
            attempt.state
        );

        let result = attempt.finish(&copied_link).unwrap();
        assert_eq!(result.session_token_code, "session-code");
    }

    #[test]
    fn accepts_selection_parameters_in_url_fragment() {
        let attempt = begin_login().unwrap();
        let copied_link = format!(
            "https://accounts.nintendo.com/connect/1.0.0/authorize#de=session-code&st={}",
            attempt.state
        );

        let result = attempt.finish(&copied_link).unwrap();
        assert_eq!(result.session_token_code, "session-code");
    }

    #[test]
    fn accepts_custom_scheme_fragment() {
        let attempt = begin_login().unwrap();
        let copied_link = format!(
            "{REDIRECT_URI}#session_token_code=eyJhbGciOiJIUzI1NiJ9.fake-token&state={}&session_state=opaque",
            attempt.state
        );

        let result = attempt.finish(&copied_link).unwrap();
        assert_eq!(result.session_token_code, "eyJhbGciOiJIUzI1NiJ9.fake-token");
    }
}
