use std::collections::HashSet;
use crate::util::get_bittorrent_api_url;
use crate::Configuration;
use reqwest::Client;
use std::ops::Deref;
use std::sync::Arc;
use log::info;
use telers::client::Reqwest;
use telers::errors::EventErrorKind;
use telers::event::EventReturn;
use telers::middlewares::outer::MiddlewareResponse;
use telers::middlewares::OuterMiddleware;
use telers::router::Request;
use telers::FromContext;

#[derive(Clone, FromContext)]
#[context(key = "state")]
pub struct BotState {
    inner: Arc<Inner>,
}

pub struct Inner {
    pub client: Client,
    pub options: BitTorrentClientOptions,
    allowed_user_ids: HashSet<i64>,
}

pub struct BitTorrentClientOptions {
    pub url: String,
    pub username: String,
    pub password: String,
}

impl BotState {
    pub fn new(configuration: Configuration) -> anyhow::Result<Self> {
        let allowed_user_ids: HashSet<i64> = configuration.user_id.split(',')
            .map(|id| Ok::<i64, anyhow::Error>(id.parse::<i64>()?))
            .filter_map(Result::ok)
            .collect();

        info!("allowed user_ids: {:?}", allowed_user_ids);

        Ok(Self {
            inner: Arc::new(Inner {
                allowed_user_ids,
                client: Client::new(),
                options: BitTorrentClientOptions {
                    password: configuration.password.clone(),
                    username: configuration.username.clone(),
                    url: get_bittorrent_api_url(&configuration)?,
                },
            }),
        })
    }

    pub fn user_allowed(&self, user_id: i64) -> bool {
        self.inner.allowed_user_ids.contains(&user_id)
    }
}

impl Deref for BotState {
    type Target = Inner;

    fn deref(&self) -> &Self::Target {
        &self.inner
    }
}

#[async_trait::async_trait]
impl OuterMiddleware for BotState {
    async fn call(
        &self,
        request: Request<Reqwest>,
    ) -> Result<MiddlewareResponse<Reqwest>, EventErrorKind> {
        request.context.insert("state", Box::new(self.clone()));
        Ok((request, EventReturn::default()))
    }
}
