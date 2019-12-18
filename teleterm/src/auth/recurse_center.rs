use crate::prelude::*;

pub fn oauth_config(
    client_id: &str,
    client_secret: &str,
    redirect_url: &url::Url,
) -> crate::oauth::Config {
    crate::oauth::Config::new(
        client_id.to_string(),
        client_secret.to_string(),
        url::Url::parse("https://www.recurse.com/oauth/authorize").unwrap(),
        url::Url::parse("https://www.recurse.com/oauth/token").unwrap(),
        redirect_url.clone(),
    )
}

pub fn get_username(
    access_token: &str,
) -> Box<dyn futures::Future<Item = String, Error = Error> + Send> {
    let fut = reqwest::r#async::Client::new()
        .get("https://www.recurse.com/api/v1/profiles/me")
        .bearer_auth(access_token)
        .send()
        .context(crate::error::GetRecurseCenterProfile)
        .and_then(|mut res| res.json().context(crate::error::ParseJson))
        .map(|user: User| user.name());
    Box::new(fut)
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
