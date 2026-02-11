use grammers_client::types::LoginToken;
use grammers_client::{Client, SignInError};

use super::CollectorError;

/// Request a login code for the given phone number.
pub async fn request_login_code(
    client: &Client,
    phone: &str,
    api_hash: &str,
) -> Result<LoginToken, CollectorError> {
    client
        .request_login_code(phone, api_hash)
        .await
        .map_err(|e| CollectorError::Auth(format!("failed to request login code: {}", e)))
}

/// Sign in with the received code.
/// Returns Ok(true) if signed in, Ok(false) if 2FA is required.
pub async fn sign_in(
    client: &Client,
    token: &LoginToken,
    code: &str,
) -> Result<SignInResult, CollectorError> {
    match client.sign_in(token, code).await {
        Ok(_user) => Ok(SignInResult::Success),
        Err(SignInError::PasswordRequired(password_token)) => {
            let hint = password_token.hint().unwrap_or("none").to_string();
            Ok(SignInResult::TwoFactorRequired {
                password_token: Box::new(password_token),
                hint,
            })
        }
        Err(e) => Err(CollectorError::Auth(format!("sign in failed: {}", e))),
    }
}

/// Complete 2FA sign-in with the password.
pub async fn check_password(
    client: &Client,
    password_token: grammers_client::types::PasswordToken,
    password: &str,
) -> Result<(), CollectorError> {
    client
        .check_password(password_token, password)
        .await
        .map_err(|e| CollectorError::Auth(format!("2FA failed: {}", e)))?;
    Ok(())
}

/// Check if the client is already authorized (has a valid session).
pub async fn is_authorized(client: &Client) -> Result<bool, CollectorError> {
    client
        .is_authorized()
        .await
        .map_err(|e| CollectorError::Auth(format!("auth check failed: {}", e)))
}

pub enum SignInResult {
    Success,
    TwoFactorRequired {
        password_token: Box<grammers_client::types::PasswordToken>,
        hint: String,
    },
}
