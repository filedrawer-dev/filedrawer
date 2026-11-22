//! GitHub sign-in's calls to GitHub (design.md, Accounts; #119): the OAuth App's
//! authorize URL, and the `GithubOAuth` seam that turns the code GitHub sends back
//! into who the person is on GitHub. The token it gets is revoked at once, so the
//! server never holds one.

use std::time::Duration;

use base64::Engine;
use serde::Deserialize;
use sha2::{Digest, Sha256};

/// An identification under way.
pub type Identifying<'a> =
    std::pin::Pin<Box<dyn std::future::Future<Output = Result<Identity, Failure>> + Send + 'a>>;

/// The only scope asked for: reading the account's email addresses, with whether
/// GitHub verified each (the owner's decision, 2026-11-22). The public profile needs
/// none.
pub const SCOPE: &str = "user:email";

/// Where GitHub's sign-in pages are.
pub const GITHUB_WEB: &str = "https://github.com";

/// The callback's path, which each environment's OAuth App names exactly.
pub const CALLBACK_PATH: &str = "/login/github/callback";

/// How long each call to GitHub may take.
const TIMEOUT: Duration = Duration::from_secs(10);

/// Who the hosts see calling.
const USER_AGENT: &str = "filedrawer.dev sign-in (https://filedrawer.dev/skill.md)";

/// Addresses GitHub makes for people who keep theirs private, which get no mail.
const NOREPLY_DOMAIN: &str = "@users.noreply.github.com";

/// An OAuth App's client secret, which Debug output leaves out.
#[derive(Clone)]
pub struct ClientSecret(pub String);

impl std::fmt::Debug for ClientSecret {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("ClientSecret(..)")
    }
}

/// An environment's OAuth App (`FILEDRAWER_GITHUB_CLIENT_ID` and `_SECRET`).
#[derive(Debug, Clone)]
pub struct OAuthApp {
    pub client_id: String,
    pub client_secret: ClientSecret,
}

/// Who signed in, as GitHub said.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Identity {
    /// GitHub's numeric user ID, which outlives a rename.
    pub github_id: i64,
    pub login: String,
    /// The profile's name, if it has one.
    pub name: Option<String>,
    /// When the GitHub account was made, as the server stamps times; none when the
    /// answer held none the server could read.
    pub created_at: Option<String>,
    /// The verified addresses that get mail, the primary one first.
    pub emails: Vec<String>,
}

/// Why a sign-in found no identity.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Failure {
    /// GitHub refused the code: used, expired, or not for this app.
    CodeRefused(String),
    /// The person didn't grant `user:email`, so GitHub won't list their addresses.
    NoEmailAccess,
    /// GitHub didn't answer, or answered in a shape the server can't read.
    Unavailable(String),
}

/// How the server learns who a code belongs to, so tests can stand in for GitHub.
pub trait GithubOAuth: Send + Sync {
    /// Exchanges `code`, with the PKCE `verifier` its start made and the exact
    /// `redirect_uri` the start sent, for a token; reads the user and their
    /// addresses with it; then revokes it.
    fn identify<'a>(
        &'a self,
        code: &'a str,
        verifier: &'a str,
        redirect_uri: &'a str,
    ) -> Identifying<'a>;
}

/// GitHub sign-in as an environment sets it up: its OAuth App and how to reach
/// GitHub.
pub struct GithubLogin {
    pub app: OAuthApp,
    pub oauth: std::sync::Arc<dyn GithubOAuth>,
    /// `https://github.com`, or a test's stand-in, for the authorize URL.
    pub web_base: String,
}

impl GithubLogin {
    /// The URL that sends the browser to GitHub's consent page. GitHub always shows
    /// its account picker, so a browser signed in to someone else's GitHub account
    /// can't link it unseen.
    pub fn authorize_url(&self, redirect_uri: &str, state: &str, verifier: &str) -> String {
        let query: String = url::form_urlencoded::Serializer::new(String::new())
            .append_pair("client_id", &self.app.client_id)
            .append_pair("redirect_uri", redirect_uri)
            .append_pair("scope", SCOPE)
            .append_pair("state", state)
            .append_pair("code_challenge", &challenge(verifier))
            .append_pair("code_challenge_method", "S256")
            .append_pair("prompt", "select_account")
            .finish();
        format!("{}/login/oauth/authorize?{query}", self.web_base)
    }
}

/// PKCE's S256 challenge for `verifier` (RFC 7636): its SHA-256, base64url without
/// padding.
pub fn challenge(verifier: &str) -> String {
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()))
}

/// Reaches GitHub over HTTPS, never following a redirect.
pub struct HttpGithubOAuth {
    http: reqwest::Client,
    app: OAuthApp,
    web_base: String,
    api_base: String,
}

impl HttpGithubOAuth {
    pub fn new(app: OAuthApp) -> HttpGithubOAuth {
        HttpGithubOAuth::with_bases(app, GITHUB_WEB, crate::link_checks::GITHUB_API, TIMEOUT)
    }

    /// With stand-ins for `https://github.com` and `https://api.github.com`, for tests.
    pub fn with_bases(
        app: OAuthApp,
        web_base: &str,
        api_base: &str,
        timeout: Duration,
    ) -> HttpGithubOAuth {
        let http = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .timeout(timeout)
            .user_agent(USER_AGENT)
            .build()
            .expect("a client with only a redirect policy, a timeout and an agent builds");
        HttpGithubOAuth {
            http,
            app,
            web_base: web_base.trim_end_matches('/').to_string(),
            api_base: api_base.trim_end_matches('/').to_string(),
        }
    }

    async fn token(
        &self,
        code: &str,
        verifier: &str,
        redirect_uri: &str,
    ) -> Result<String, Failure> {
        let answer: TokenAnswer = self
            .http
            .post(format!("{}/login/oauth/access_token", self.web_base))
            .header(reqwest::header::ACCEPT, "application/json")
            .form(&[
                ("client_id", self.app.client_id.as_str()),
                ("client_secret", self.app.client_secret.0.as_str()),
                ("code", code),
                ("redirect_uri", redirect_uri),
                ("code_verifier", verifier),
            ])
            .send()
            .await
            .map_err(unavailable)?
            .error_for_status()
            .map_err(unavailable)?
            .json()
            .await
            .map_err(unavailable)?;
        // GitHub answers a refused code with HTTP 200 and an `error`.
        match answer {
            TokenAnswer {
                access_token: Some(token),
                ..
            } => Ok(token),
            TokenAnswer {
                error: Some(error), ..
            } if error == "incorrect_client_credentials" || error == "redirect_uri_mismatch" => {
                Err(Failure::Unavailable(format!(
                    "GitHub refused the app's settings: {error}"
                )))
            }
            TokenAnswer {
                error: Some(error), ..
            } => Err(Failure::CodeRefused(error)),
            TokenAnswer { .. } => Err(Failure::Unavailable("GitHub sent no token".into())),
        }
    }

    fn api(&self, token: &str, path: &str) -> reqwest::RequestBuilder {
        self.http
            .get(format!("{}{path}", self.api_base))
            .bearer_auth(token)
            .header(reqwest::header::ACCEPT, "application/vnd.github+json")
            .header("x-github-api-version", "2022-11-28")
    }

    async fn user(&self, token: &str) -> Result<User, Failure> {
        self.api(token, "/user")
            .send()
            .await
            .map_err(unavailable)?
            .error_for_status()
            .map_err(unavailable)?
            .json()
            .await
            .map_err(unavailable)
    }

    async fn emails(&self, token: &str) -> Result<Vec<Email>, Failure> {
        let response = self
            .api(token, "/user/emails")
            .send()
            .await
            .map_err(unavailable)?;
        // Without `user:email`, GitHub answers 404 here; its 403 is a rate limit,
        // which `error_for_status` makes a failure to answer.
        if response.status() == reqwest::StatusCode::NOT_FOUND {
            return Err(Failure::NoEmailAccess);
        }
        response
            .error_for_status()
            .map_err(unavailable)?
            .json()
            .await
            .map_err(unavailable)
    }

    /// Revokes `token`, so the server holds no working one. A failure is logged: the
    /// token was never stored.
    async fn revoke(&self, token: &str) {
        let revoked = self
            .http
            .delete(format!(
                "{}/applications/{}/token",
                self.api_base, self.app.client_id
            ))
            .basic_auth(&self.app.client_id, Some(&self.app.client_secret.0))
            .header(reqwest::header::ACCEPT, "application/vnd.github+json")
            .json(&serde_json::json!({ "access_token": token }))
            .send()
            .await
            .and_then(reqwest::Response::error_for_status);
        if let Err(error) = revoked {
            tracing::warn!(error = %error.without_url(), "revoking a GitHub sign-in token failed");
        }
    }
}

impl GithubOAuth for HttpGithubOAuth {
    fn identify<'a>(
        &'a self,
        code: &'a str,
        verifier: &'a str,
        redirect_uri: &'a str,
    ) -> Identifying<'a> {
        Box::pin(async move {
            let token = self.token(code, verifier, redirect_uri).await?;
            let read = async {
                let user = self.user(&token).await?;
                let emails = self.emails(&token).await?;
                Ok::<_, Failure>((user, emails))
            }
            .await;
            self.revoke(&token).await;
            let (user, emails) = read?;
            Ok(identity(user, emails))
        })
    }
}

#[derive(Deserialize)]
struct TokenAnswer {
    access_token: Option<String>,
    error: Option<String>,
}

#[derive(Deserialize)]
struct User {
    id: i64,
    login: String,
    name: Option<String>,
    created_at: Option<String>,
}

#[derive(Deserialize)]
struct Email {
    email: String,
    verified: bool,
    primary: bool,
}

fn unavailable(error: reqwest::Error) -> Failure {
    Failure::Unavailable(if error.is_timeout() {
        "GitHub didn't answer in time".into()
    } else {
        error.without_url().to_string()
    })
}

/// What the server keeps of GitHub's answers: the verified addresses that get mail,
/// the primary first, and the account's age as the server stamps times. A time it
/// can't read is none, which grants no exemption, and is logged, since it means
/// GitHub's answer changed shape.
fn identity(user: User, emails: Vec<Email>) -> Identity {
    let mut usable: Vec<&Email> = emails
        .iter()
        .filter(|email| {
            email.verified && !email.email.to_ascii_lowercase().ends_with(NOREPLY_DOMAIN)
        })
        .collect();
    usable.sort_by_key(|email| !email.primary);
    let created_at = user.created_at.as_deref().and_then(|text| {
        let parsed =
            time::OffsetDateTime::parse(text, &time::format_description::well_known::Rfc3339);
        if parsed.is_err() {
            tracing::warn!(created_at = text, "GitHub's created_at isn't RFC 3339");
        }
        parsed.ok().map(crate::time::stamp)
    });
    if user.created_at.is_none() {
        tracing::warn!(github_id = user.id, "GitHub's answer has no created_at");
    }
    Identity {
        github_id: user.id,
        login: user.login,
        name: user.name.filter(|name| !name.trim().is_empty()),
        created_at,
        emails: usable
            .into_iter()
            .map(|email| crate::accounts::normalize_email(&email.email))
            .filter(|email| crate::accounts::looks_like_email(email))
            .collect(),
    }
}
