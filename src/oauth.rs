use crate::prelude::*;

pub mod recurse_center;

pub trait Oauth {
    fn client(&self) -> &oauth2::basic::BasicClient;

    fn generate_authorize_url(&self) -> String {
        let (auth_url, _) = self
            .client()
            .authorize_url(oauth2::CsrfToken::new_random)
            .url();
        auth_url.to_string()
    }

    fn get_token(
        &self,
        code: &str,
    ) -> Box<
        dyn futures::future::Future<
                Item = oauth2::basic::BasicTokenResponse,
                Error = Error,
            > + Send,
    > {
        let fut = self
            .client()
            .exchange_code(oauth2::AuthorizationCode::new(code.to_string()))
            .request_async(oauth2::reqwest::async_http_client)
            .map_err(|e| {
                let msg = stringify_oauth2_http_error(&e);
                Error::ExchangeCode { msg }
            });
        Box::new(fut)
    }

    fn get_username(
        &self,
        code: &str,
    ) -> Box<dyn futures::future::Future<Item = String, Error = Error> + Send>;
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
