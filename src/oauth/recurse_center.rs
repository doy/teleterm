use crate::prelude::*;

pub struct RecurseCenter {
    client: oauth2::basic::BasicClient,
    user_id: String,
}

impl RecurseCenter {
    pub fn new(
        client_id: &str,
        client_secret: &str,
        redirect_url: url::Url,
        user_id: &str,
    ) -> Self {
        let config = super::Config {
            client_id: client_id.to_string(),
            client_secret: client_secret.to_string(),
            auth_url: url::Url::parse(
                "https://www.recurse.com/oauth/authorize",
            )
            .unwrap(),
            token_url: url::Url::parse("https://www.recurse.com/oauth/token")
                .unwrap(),
            redirect_url,
        };

        Self {
            client: config.into_basic_client(),
            user_id: user_id.to_string(),
        }
    }
}

impl super::Oauth for RecurseCenter {
    fn client(&self) -> &oauth2::basic::BasicClient {
        &self.client
    }

    fn user_id(&self) -> &str {
        &self.user_id
    }

    fn name(&self) -> &str {
        crate::protocol::AuthType::RecurseCenter.name()
    }

    fn get_username_from_access_token(
        self: Box<Self>,
        token: &str,
    ) -> Box<dyn futures::future::Future<Item = String, Error = Error> + Send>
    {
        let fut = reqwest::r#async::Client::new()
            .get("https://www.recurse.com/api/v1/profiles/me")
            .bearer_auth(token)
            .send()
            .context(crate::error::GetProfile)
            .and_then(|mut res| res.json().context(crate::error::ParseJson))
            .map(|user: User| user.name());
        Box::new(fut)
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
