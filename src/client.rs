use anyhow::anyhow;
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

    pub async fn application_version(&self) -> anyhow::Result<String> {
        let app_ver_url = self.build_url("/api/v2/app/version").await?;
        let res = self.http_client.get(app_ver_url).send().await?;

        Ok(res.text().await?)
    }

    pub async fn add_new_torrent_with_magnet(&self, magnet: &str) -> anyhow::Result<()> {
        let url = self.build_url("/api/v2/torrents/add").await?;

        let bytes = magnet.as_bytes();
        let multipart = multipart::Form::new().part("urls", Part::bytes(bytes.to_vec()));

        let res = self
            .http_client
            .post(url)
            .multipart(multipart)
            .send()
            .await?;

        if !res.status().is_success() {
            return Err(anyhow!(
                "Error adding new torrent with magnet {:?}",
                res.text().await?
            ));
        }

        let text = res.text().await?;
        if text != "Ok." {
            return Err(anyhow!("Error adding new torrent with magnet, not Ok."));
        }

        Ok(())
    }
}
