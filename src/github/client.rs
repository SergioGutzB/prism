use anyhow::Result;
use reqwest::{Client, header};

pub struct GitHubClient {
    pub(crate) client: Client,
    pub(crate) base_url: String,
    pub(crate) owner: String,
    pub(crate) repo: String,
}

impl GitHubClient {
    pub fn new(token: &str, owner: &str, repo: &str) -> Result<Self> {
        let mut headers = header::HeaderMap::new();

        let auth_value = format!("Bearer {}", token);
        let mut auth_header = header::HeaderValue::from_str(&auth_value)
            .map_err(|e| anyhow::anyhow!("Invalid auth header: {}", e))?;
        auth_header.set_sensitive(true);
        headers.insert(header::AUTHORIZATION, auth_header);

        headers.insert(
            header::ACCEPT,
            header::HeaderValue::from_static("application/vnd.github+json"),
        );
        headers.insert(
            "X-GitHub-Api-Version",
            header::HeaderValue::from_static("2022-11-28"),
        );
        headers.insert(
            header::USER_AGENT,
            header::HeaderValue::from_static("prism/0.1.0"),
        );

        let client = Client::builder()
            .default_headers(headers)
            .use_rustls_tls()
            .build()?;

        Ok(Self {
            client,
            base_url: "https://api.github.com".to_string(),
            owner: owner.to_string(),
            repo: repo.to_string(),
        })
    }

    /// For testing with a custom base URL (e.g., mockito).
    #[cfg(test)]
    pub fn with_base_url(mut self, base_url: &str) -> Self {
        self.base_url = base_url.to_string();
        self
    }
}
