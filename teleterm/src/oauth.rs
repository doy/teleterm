use crate::prelude::*;
use oauth2::TokenResponse as _;

// this needs to be fixed because we listen for it in a hardcoded place
pub const CLI_REDIRECT_URL: &str = "http://localhost:44141/oauth";

pub struct Oauth {
    client: oauth2::basic::BasicClient,
    user_id: String,
}

impl Oauth {
    pub fn new(config: Config, user_id: String) -> Self {
        let client = config.into_basic_client();
        Self { client, user_id }
    }

    pub fn generate_authorize_url(&self) -> String {
        let (auth_url, _) = self
            .client
            .authorize_url(oauth2::CsrfToken::new_random)
            .url();
        auth_url.to_string()
    }

    pub fn user_id(&self) -> &str {
        &self.user_id
    }

    pub fn get_access_token_from_auth_code(
        &self,
        code: &str,
    ) -> Box<dyn futures::Future<Item = String, Error = Error> + Send> {
        let token_cache_file = self.server_token_file(false).unwrap();
        let fut = self
            .client
            .exchange_code(oauth2::AuthorizationCode::new(code.to_string()))
            .request_future(oauth2::reqwest::future_http_client)
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

    pub fn get_access_token_from_refresh_token(
        self,
    ) -> Box<dyn futures::Future<Item = String, Error = Error> + Send> {
        let token_cache_file = self.server_token_file(false).unwrap();
        let fut = load_refresh_token(&token_cache_file).and_then(
            move |refresh_token| {
                // XXX
                let refresh_token = refresh_token.unwrap();
                self.client
                    .exchange_refresh_token(&oauth2::RefreshToken::new(
                        refresh_token,
                    ))
                    .request_future(oauth2::reqwest::future_http_client)
                    .map_err(|e| {
                        let msg = stringify_oauth2_http_error(&e);
                        Error::ExchangeRefreshToken { msg }
                    })
                    .and_then(move |token| {
                        cache_refresh_token(token_cache_file, &token).map(
                            move |_| {
                                token.access_token().secret().to_string()
                            },
                        )
                    })
            },
        );
        Box::new(fut)
    }

    pub fn server_token_file(
        &self,
        must_exist: bool,
    ) -> Option<std::path::PathBuf> {
        let name = format!("server-oauth-{}", self.user_id);
        crate::dirs::Dirs::new().data_file(&name, must_exist)
    }
}

fn load_refresh_token(
    token_cache_file: &std::path::Path,
) -> Box<dyn futures::Future<Item = Option<String>, Error = Error> + Send> {
    let token_cache_file = token_cache_file.to_path_buf();
    Box::new(
        tokio::fs::File::open(token_cache_file.clone())
            .with_context(move || crate::error::OpenFile {
                filename: token_cache_file.to_string_lossy().to_string(),
            })
            .and_then(|file| {
                tokio::io::lines(std::io::BufReader::new(file))
                    .into_future()
                    .map_err(|(e, _)| e)
                    .context(crate::error::ReadFile)
            })
            .map(|(refresh_token, _)| refresh_token),
    )
}

fn cache_refresh_token(
    token_cache_file: std::path::PathBuf,
    token: &oauth2::basic::BasicTokenResponse,
) -> Box<dyn futures::Future<Item = (), Error = Error> + Send> {
    let token_data = format!(
        "{}\n{}\n",
        token.refresh_token().unwrap().secret(),
        token.access_token().secret(),
    );
    let fut = tokio::fs::File::create(token_cache_file.clone())
        .with_context(move || crate::error::CreateFile {
            filename: token_cache_file.to_string_lossy().to_string(),
        })
        .and_then(|file| {
            tokio::io::write_all(file, token_data)
                .context(crate::error::WriteFile)
        })
        .map(|_| ());
    Box::new(fut)
}

#[derive(Debug, Clone)]
pub struct Config {
    client_id: String,
    client_secret: String,
    auth_url: url::Url,
    token_url: url::Url,
    redirect_url: url::Url,
}

impl Config {
    pub fn new(
        client_id: String,
        client_secret: String,
        auth_url: url::Url,
        token_url: url::Url,
        redirect_url: url::Url,
    ) -> Self {
        Self {
            client_id,
            client_secret,
            auth_url,
            token_url,
            redirect_url,
        }
    }

    pub fn set_redirect_url(&mut self, url: url::Url) {
        self.redirect_url = url;
    }

    fn into_basic_client(self) -> oauth2::basic::BasicClient {
        oauth2::basic::BasicClient::new(
            oauth2::ClientId::new(self.client_id),
            Some(oauth2::ClientSecret::new(self.client_secret)),
            oauth2::AuthUrl::new(self.auth_url.to_string()).unwrap(),
            Some(oauth2::TokenUrl::new(self.token_url.to_string()).unwrap()),
        )
        .set_redirect_url(
            oauth2::RedirectUrl::new(self.redirect_url.to_string()).unwrap(),
        )
    }
}

// make this actually give useful information, because the default
// stringification is pretty useless
fn stringify_oauth2_http_error(
    e: &oauth2::RequestTokenError<
        oauth2::reqwest::Error<reqwest::Error>,
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
