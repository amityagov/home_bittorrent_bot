use anyhow::anyhow;
use bytes::Bytes;
use reqwest::multipart::Part;
use reqwest::{multipart, Client, Url};
use tokio::sync::RwLock;

pub struct QBittorrentClient {
    http_client: Client,
    base_url: RwLock<Url>,
}

impl QBittorrentClient {
    pub async fn new<S: ToString>(url: S) -> anyhow::Result<Self> {
        let http_client = Client::builder().cookie_store(true).build()?;

        let base_url = Url::parse(&url.to_string())?;
        let base_url = RwLock::new(base_url);

        Ok(QBittorrentClient {
            http_client,
            base_url,
        })
    }

    async fn build_url(&self, endpoint: &str) -> anyhow::Result<Url> {
        let base_url = self.base_url.read().await;
        Ok(base_url.join(endpoint)?)
    }

    pub async fn login<S: ToString>(&self, username: S, password: S) -> anyhow::Result<()> {
        let base_url = self.base_url.read().await;
        let login_url = base_url.join("/api/v2/auth/login")?;

        let res = self
            .http_client
            .post(login_url)
            .header("Content-Type", "application/x-www-form-urlencoded")
            .header("Referer", base_url.to_string())
            .body(format!(
                "username={}&password={}",
                username.to_string(),
                password.to_string()
            ))
            .send()
            .await?;

        if res.status().is_success() {
            Ok(())
        } else {
            Err(anyhow!("Auth failed"))
        }
    }

    pub async fn add_new_torrent<'a>(&self, request_type: RequestType<'a>) -> anyhow::Result<()> {
        let url = self.build_url("/api/v2/torrents/add").await?;

        let (name, part) = request_type.to_part()?;
        let multipart = multipart::Form::new().part(name, part);

        let res = self
            .http_client
            .post(url)
            .multipart(multipart)
            .send()
            .await?;

        if !res.status().is_success() {
            return Err(anyhow!(
                "Error adding new torrent with file {:?}",
                res.text().await?
            ));
        }

        let text = res.text().await?;
        if text != "Ok." {
            return Err(anyhow!(
                "Error adding new torrent with file, not Ok., but {}",
                text
            ));
        }

        Ok(())
    }
}

pub enum RequestType<'a> {
    Url(&'a str),
    File(&'a Bytes),
}

impl<'a> RequestType<'a> {
    fn to_part(self) -> anyhow::Result<(&'static str, Part)> {
        match self {
            RequestType::Url(url) => Ok(("urls", Part::bytes(url.as_bytes().to_vec()))),
            RequestType::File(file) => Ok((
                "torrents",
                Part::bytes(file.to_vec())
                    .file_name("torrent.torrent")
                    .mime_str("application/x-bittorrent")?,
            )),
        }
    }
}
