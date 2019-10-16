use crate::prelude::*;
use oauth2::TokenResponse as _;

mod recurse_center;
pub use recurse_center::RecurseCenter;

pub trait Oauth {
    fn client(&self) -> &oauth2::basic::BasicClient;
    fn user_id(&self) -> &str;
    fn name(&self) -> &str;
    fn token_cache_file(&self) -> std::path::PathBuf {
        let name = format!("server-oauth-{}-{}", self.name(), self.user_id());
        crate::dirs::Dirs::new().cache_file(&name)
    }

    fn generate_authorize_url(&self) -> String {
        let (auth_url, _) = self
            .client()
            .authorize_url(oauth2::CsrfToken::new_random)
            .url();
        auth_url.to_string()
    }

    fn get_access_token_from_auth_code(
        &self,
        code: &str,
    ) -> Box<dyn futures::future::Future<Item = String, Error = Error> + Send>
    {
        let token_cache_file = self.token_cache_file();
        let fut = self
            .client()
            .exchange_code(oauth2::AuthorizationCode::new(code.to_string()))
            .request_async(oauth2::reqwest::async_http_client)
            .map_err(|e| {
                let msg = stringify_oauth2_http_error(&e);
                Error::ExchangeCode { msg }
            })
            .and_then(|token| {
                cache_refresh_token(token_cache_file, &token)
                    .map(move |_| token.access_token().secret().to_string())
            });
        Box::new(fut)
    }

    fn get_access_token_from_refresh_token(
        &self,
        token: &str,
    ) -> Box<dyn futures::future::Future<Item = String, Error = Error> + Send>
    {
        let token_cache_file = self.token_cache_file();
        let fut = self
            .client()
            .exchange_refresh_token(&oauth2::RefreshToken::new(
                token.to_string(),
            ))
            .request_async(oauth2::reqwest::async_http_client)
            .map_err(|e| {
                let msg = stringify_oauth2_http_error(&e);
                Error::ExchangeCode { msg }
            })
            .and_then(|token| {
                cache_refresh_token(token_cache_file, &token)
                    .map(move |_| token.access_token().secret().to_string())
            });
        Box::new(fut)
    }

    fn get_username_from_access_token(
        self: Box<Self>,
        token: &str,
    ) -> Box<dyn futures::future::Future<Item = String, Error = Error> + Send>;
}

fn cache_refresh_token(
    token_cache_file: std::path::PathBuf,
    token: &oauth2::basic::BasicTokenResponse,
) -> Box<dyn futures::future::Future<Item = (), Error = Error> + Send> {
    let token_data = format!(
        "{}\n{}\n",
        token.refresh_token().unwrap().secret(),
        token.access_token().secret(),
    );
    let fut = tokio::fs::File::create(token_cache_file)
        .context(crate::error::CreateFile)
        .and_then(|file| {
            tokio::io::write_all(file, token_data)
                .context(crate::error::WriteFile)
        })
        .map(|_| ());
    Box::new(fut)
}

pub struct Config {
    client_id: String,
    client_secret: String,
    auth_url: url::Url,
    token_url: url::Url,
    redirect_url: url::Url,
}

impl Config {
    fn into_basic_client(self) -> oauth2::basic::BasicClient {
        oauth2::basic::BasicClient::new(
            oauth2::ClientId::new(self.client_id),
            Some(oauth2::ClientSecret::new(self.client_secret)),
            oauth2::AuthUrl::new(self.auth_url),
            Some(oauth2::TokenUrl::new(self.token_url)),
        )
        .set_redirect_url(oauth2::RedirectUrl::new(self.redirect_url))
    }
}

// make this actually give useful information, because the default
// stringification is pretty useless
fn stringify_oauth2_http_error(
    e: &oauth2::RequestTokenError<
        oauth2::reqwest::Error,
        oauth2::StandardErrorResponse<oauth2::basic::BasicErrorResponseType>,
    >,
) -> String {
    match e {
        oauth2::RequestTokenError::ServerResponse(t) => {
            format!("ServerResponse({})", t)
        }
        oauth2::RequestTokenError::Request(re) => format!("Request({})", re),
        oauth2::RequestTokenError::Parse(se, b) => format!(
            "Parse({}, {})",
            se,
            std::string::String::from_utf8_lossy(b)
        ),
        oauth2::RequestTokenError::Other(s) => format!("Other({})", s),
    }
}
