use crate::prelude::*;
use oauth2::TokenResponse as _;

pub struct Oauth {
    client: oauth2::basic::BasicClient,
}

impl Oauth {
    pub fn new(config: super::Config) -> Self {
        Self {
            client: config.into_basic_client(),
        }
    }
}

impl super::Oauth for Oauth {
    fn client(&self) -> &oauth2::basic::BasicClient {
        &self.client
    }

    fn get_username(
        &self,
        code: &str,
    ) -> Box<dyn futures::future::Future<Item = String, Error = Error> + Send>
    {
        let fut = self
            .get_token(code)
            .and_then(|token| {
                let access_token = token.access_token();
                reqwest::r#async::Client::new()
                    .get("https://www.recurse.com/api/v1/profiles/me")
                    .bearer_auth(access_token.secret())
                    .send()
                    .context(crate::error::GetProfile)
            })
            .and_then(|mut res| res.json().context(crate::error::ParseJson))
            .map(|user: User| user.name());
        Box::new(fut)
    }
}

pub fn config(
    client_id: &str,
    client_secret: &str,
    redirect_url: url::Url,
) -> super::Config {
    super::Config {
        client_id: client_id.to_string(),
        client_secret: client_secret.to_string(),
        auth_url: url::Url::parse("https://www.recurse.com/oauth/authorize")
            .unwrap(),
        token_url: url::Url::parse("https://www.recurse.com/oauth/token")
            .unwrap(),
        redirect_url,
    }
}

#[derive(serde::Deserialize)]
struct User {
    name: String,
    stints: Vec<Stint>,
}

#[derive(serde::Deserialize)]
struct Stint {
    batch: Option<Batch>,
    start_date: String,
}

#[derive(serde::Deserialize)]
struct Batch {
    short_name: String,
}

impl User {
    fn name(&self) -> String {
        let latest_stint =
            self.stints.iter().max_by_key(|s| &s.start_date).unwrap();
        if let Some(batch) = &latest_stint.batch {
            format!("{} ({})", self.name, batch.short_name)
        } else {
            self.name.to_string()
        }
    }
}
