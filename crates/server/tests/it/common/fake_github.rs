//! A fake `GithubOAuth`: GitHub as a test sets it up, so no test reaches GitHub.
//! Each code a test hands out names the identity, or the failure, GitHub gives for
//! it; every exchange is recorded.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use filedrawer_server::github_login::{
    ClientSecret, Failure, GithubLogin, GithubOAuth, Identifying, Identity, OAuthApp,
};

/// The test OAuth App's client ID.
pub const CLIENT_ID: &str = "Iv1.testapp";
/// Where the fake's authorize page is.
pub const WEB: &str = "https://github.test";

#[derive(Clone, Default)]
pub struct FakeGithub {
    codes: Arc<Mutex<HashMap<String, Result<Identity, Failure>>>>,
    /// Each exchange: the code, the verifier and the redirect URI.
    pub exchanges: Arc<Mutex<Vec<(String, String, String)>>>,
}

impl FakeGithub {
    /// `code` identifies `identity`.
    pub fn code_for(&self, code: &str, identity: Identity) {
        self.codes.lock().unwrap().insert(code.into(), Ok(identity));
    }

    /// `code` fails with `failure`.
    pub fn code_fails(&self, code: &str, failure: Failure) {
        self.codes.lock().unwrap().insert(code.into(), Err(failure));
    }

    pub fn login(&self) -> Arc<GithubLogin> {
        Arc::new(GithubLogin {
            app: OAuthApp {
                client_id: CLIENT_ID.into(),
                client_secret: ClientSecret("test-secret".into()),
            },
            oauth: Arc::new(self.clone()),
            web_base: WEB.into(),
        })
    }
}

impl GithubOAuth for FakeGithub {
    fn identify<'a>(
        &'a self,
        code: &'a str,
        verifier: &'a str,
        redirect_uri: &'a str,
    ) -> Identifying<'a> {
        Box::pin(async move {
            self.exchanges.lock().unwrap().push((
                code.into(),
                verifier.into(),
                redirect_uri.into(),
            ));
            // A code works once, as GitHub's do.
            self.codes
                .lock()
                .unwrap()
                .remove(code)
                .unwrap_or_else(|| Err(Failure::CodeRefused("bad_verification_code".into())))
        })
    }
}

/// A GitHub account made in 2015, with one verified address.
pub fn octo(id: i64, login: &str, email: &str) -> Identity {
    Identity {
        github_id: id,
        login: login.into(),
        name: Some("Octo Cat".into()),
        created_at: Some("2015-01-01T00:00:00Z".into()),
        emails: vec![email.into()],
    }
}
