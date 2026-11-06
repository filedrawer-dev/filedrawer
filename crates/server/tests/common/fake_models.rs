//! A fake `ModelClient` that answers by markers in the screened text, and records
//! which models it was asked.

use std::sync::{Arc, Mutex};

use filedrawer_server::screening::{
    Answer, Calling, Model, ModelClient, ModelError, ModelReply, ModelRequest, Screener, Usage,
};
use serde_json::json;

/// What each call reports it used.
pub const USAGE: Usage = Usage {
    input_tokens: 1_000,
    output_tokens: 200,
};

/// Answers by markers in the text:
/// - `FAKE-OFFTOPIC`, `FAKE-SPAM`, `FAKE-PIPE`: Haiku fails it for that.
/// - `FAKE-UNSURE`: Haiku is unsure of the topic. When Sonnet is asked, it says
///   off-topic only with `FAKE-SONNET-OFFTOPIC`.
/// - `FAKE-HAZARD`: Haiku thinks a hazard likely; `FAKE-HAZARD-UNSURE`: Haiku is
///   unsure. Sonnet says likely, with a category and a quote, only with
///   `FAKE-SONNET-HAZARD`; unsure with `FAKE-SONNET-UNSURE`; likely but with no
///   quote with `FAKE-SONNET-NOQUOTE`, with an empty quote with
///   `FAKE-SONNET-EMPTYQUOTE`, or with a quote but no category with
///   `FAKE-SONNET-NOCATEGORY`.
/// - `FAKE-REFUSE`: Haiku declines; Sonnet declines too with `FAKE-SONNET-REFUSE`.
/// - `FAKE-DOUBT`: a doubt about `outcome`.
/// - `FAKE-ERROR`: every call fails, as in an outage.
/// - `FAKE-GARBLED`: the reply can't be read, though it used tokens.
/// - `FAKE-SLOW`: the call waits for `release`.
#[derive(Clone)]
pub struct FakeModels {
    asked: Arc<Mutex<Vec<Model>>>,
    /// The text each Haiku call screened, in order.
    screened: Arc<Mutex<Vec<String>>>,
    /// Closed until `release`; then every slow call goes through.
    gate: Arc<tokio::sync::Semaphore>,
}

impl Default for FakeModels {
    fn default() -> Self {
        FakeModels {
            asked: Arc::default(),
            screened: Arc::default(),
            gate: Arc::new(tokio::sync::Semaphore::new(0)),
        }
    }
}

impl FakeModels {
    pub fn screener(&self) -> Screener {
        Screener::Models(Arc::new(self.clone()))
    }

    pub fn asked(&self) -> Vec<Model> {
        self.asked.lock().unwrap().clone()
    }

    /// The text each Haiku call screened, in order.
    pub fn screened(&self) -> Vec<String> {
        self.screened.lock().unwrap().clone()
    }

    /// Lets every `FAKE-SLOW` call finish, now and later.
    pub fn release(&self) {
        self.gate
            .add_permits(tokio::sync::Semaphore::MAX_PERMITS / 2);
    }

    /// Waits until `count` calls have been made.
    pub async fn wait_until_asked(&self, count: usize) {
        while self.asked().len() < count {
            tokio::time::sleep(std::time::Duration::from_millis(2)).await;
        }
    }
}

impl ModelClient for FakeModels {
    fn call<'a>(&'a self, request: &'a ModelRequest) -> Calling<'a> {
        self.asked.lock().unwrap().push(request.model);
        if request.model == Model::Haiku {
            self.screened.lock().unwrap().push(request.user.clone());
        }
        let text = &request.user;
        let has = |marker: &str| text.contains(marker);
        let sonnet = request.model == Model::Sonnet;
        let reply = if has("FAKE-ERROR") {
            Err(ModelError::Status { status: 529 })
        } else if has("FAKE-GARBLED") {
            Ok(ModelReply {
                answer: Answer::Unreadable("stop_reason max_tokens".into()),
                usage: USAGE,
            })
        } else if (!sonnet && has("FAKE-REFUSE")) || (sonnet && has("FAKE-SONNET-REFUSE")) {
            Ok(ModelReply {
                answer: Answer::Refused,
                usage: USAGE,
            })
        } else {
            let on_topic = if has("FAKE-OFFTOPIC") || (sonnet && has("FAKE-SONNET-OFFTOPIC")) {
                "no"
            } else if !sonnet && has("FAKE-UNSURE") {
                "unsure"
            } else {
                "yes"
            };
            let hazard = if sonnet {
                if has("FAKE-SONNET-HAZARD")
                    || has("FAKE-SONNET-NOQUOTE")
                    || has("FAKE-SONNET-EMPTYQUOTE")
                    || has("FAKE-SONNET-NOCATEGORY")
                {
                    "likely"
                } else if has("FAKE-SONNET-UNSURE") {
                    "unsure"
                } else {
                    "none"
                }
            } else if has("FAKE-HAZARD-UNSURE") {
                "unsure"
            } else if has("FAKE-HAZARD") {
                "likely"
            } else {
                "none"
            };
            let quote = if sonnet && has("FAKE-SONNET-EMPTYQUOTE") {
                Some("")
            } else {
                (hazard != "none" && !(sonnet && has("FAKE-SONNET-NOQUOTE")))
                    .then_some("the quoted passage")
            };
            let category = (hazard != "none" && !(sonnet && has("FAKE-SONNET-NOCATEGORY")))
                .then_some("working_attack");
            let mut reasons = Vec::new();
            if on_topic == "no" {
                reasons.push("It is about cooking, not AI safety.");
            }
            if has("FAKE-SPAM") {
                reasons.push("It advertises a product.");
            }
            Ok(ModelReply {
                answer: Answer::Json(json!({
                    "on_topic": on_topic,
                    "spam": if has("FAKE-SPAM-UNSURE") {
                        "unsure"
                    } else if has("FAKE-SPAM") {
                        "yes"
                    } else {
                        "no"
                    },
                    "hazard": hazard,
                    "hazard_category": category,
                    "hazard_quote": quote,
                    "pipes_download_to_shell": has("FAKE-PIPE"),
                    "reasons": reasons,
                    "outcome_doubt": has("FAKE-DOUBT").then_some("The body reports an effect."),
                })),
                usage: USAGE,
            })
        };
        let slow = has("FAKE-SLOW");
        let gate = self.gate.clone();
        Box::pin(async move {
            if slow {
                let _pass = gate.acquire().await;
            }
            reply
        })
    }
}
