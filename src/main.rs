mod client;

use crate::client::QBittorrentClient;
use anyhow::anyhow;
use config::{Config, Environment};
use dotenvy::dotenv;
use log::{info, trace};
use reqwest::Client;
use serde::Deserialize;
use std::io::Bytes;
use std::ops::Deref;
use std::sync::Arc;
use telers::client::Reqwest;
use telers::errors::EventErrorKind;
use telers::methods::{GetFile, SendMessage};
use telers::middlewares::outer::MiddlewareResponse;
use telers::middlewares::OuterMiddleware;
use telers::router::Request;
use telers::{
    enums::UpdateType,
    event::{telegram::HandlerResult, EventReturn, ToServiceProvider},
    types::Message,
    Bot, Dispatcher, FromContext, Router,
};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    dotenv().ok();
    env_logger::init();

    let configuration = load_config()?;

    run_bot(&configuration).await?;

    Ok(())
}

async fn run_bot(configuration: &Configuration) -> anyhow::Result<()> {
    let bot = Bot::new(configuration.bot_token.clone());

    let mut router = Router::new("main");

    let configuration = configuration.clone();
    router
        .message
        .outer_middlewares
        .register(State::new(configuration));

    router.message.register(echo_handler);

    let dispatcher = Dispatcher::builder()
        .main_router(router)
        .bot(bot)
        .allowed_update(UpdateType::Message)
        .build();

    Ok(dispatcher
        .to_service_provider_default()
        .unwrap()
        .run_polling()
        .await?)
}

async fn echo_handler(bot: Bot, message: Message, state: State) -> HandlerResult {
    trace!("got a message");

    if let Some(from) = message.from() {
        if !state.configuration.user_id.contains(&from.id.to_string()) {
            return Ok(EventReturn::Finish);
        }

        match &message {
            Message::Document(document) => {
                let file_id = &document.document.file_id;
                let file_info = bot.send(GetFile::new(file_id.deref())).await?;
                if let Some(file_path) = file_info.file_path {
                    download_torrent_file(&state, &bot.token, &file_path).await?;
                } else {
                    bot.send(SendMessage::new(
                        message.chat().id(),
                        "⛔Не удалось загрузить файл torrent",
                    ))
                        .await?;
                }
            }

            Message::Text(text) => {
                if text.text.starts_with("magnet:?") {
                    let name = magnet_url::Magnet::new(&text.text)
                        .map(|m| {
                            m.dn.map(|s| {
                                let data: String = url::form_urlencoded::parse(s.as_bytes())
                                    .map(|(k, v)| format!("{}{}", k, v))
                                    .collect();
                                data
                            })
                        })
                        .map_err(|_| anyhow!("Invalid magnet URL"))?;

                    println!("{:?}", name);

                    let text = match enqueue_download_by_magnet(&state, &text.text).await {
                        Ok(_) => format!("✅Торрент {} добавлен в очередь", name.unwrap_or_default()),
                        Err(_) => "⛔Ошибка добавления торрента, смотри логи".to_string(),
                    };

                    bot.send(SendMessage::new(message.chat().id(), text))
                        .await?;
                }
            }
            _ => {}
        }
    }

    Ok(EventReturn::Finish)
}

#[derive(Debug, Deserialize, Clone)]
struct Configuration {
    bot_token: String,
    user_id: String,
    username: String,
    password: String,
    url: String,
}

#[derive(Clone, FromContext)]
#[context(key = "state")]
struct State {
    inner: Arc<Inner>,
}

struct Inner {
    configuration: Configuration,
    client: Client,
}

impl State {
    fn new(configuration: Configuration) -> Self {
        Self {
            inner: Arc::new(Inner {
                configuration,
                client: Client::new(),
            }),
        }
    }
}

impl Deref for State {
    type Target = Inner;

    fn deref(&self) -> &Self::Target {
        &self.inner
    }
}

#[async_trait::async_trait]
impl OuterMiddleware for State {
    async fn call(
        &self,
        request: Request<Reqwest>,
    ) -> Result<MiddlewareResponse<Reqwest>, EventErrorKind> {
        request.context.insert("state", Box::new(self.clone()));
        Ok((request, EventReturn::default()))
    }
}
fn load_config() -> anyhow::Result<Configuration> {
    let config = Config::builder()
        .add_source(config::File::with_name("config"))
        .add_source(Environment::default().prefix("bittorrent_bot"))
        .build()?;

    Ok(config.try_deserialize::<Configuration>()?)
}

async fn download_torrent_file(
    state: &State,
    token: &str,
    file_path: &str,
) -> anyhow::Result<bytes::Bytes> {
    let url = format!("https://api.telegram.org/file/bot{}/{}", token, file_path);
    let response = state.client.get(url).send().await?;
    let bytes = response.bytes().await?;
    info!("downloaded file: {}", file_path);
    Ok(bytes)
}

async fn enqueue_download_by_magnet(state: &State, magnet: &str) -> anyhow::Result<()> {
    let client = QBittorrentClient::new(&state.configuration.url).await?;
    client
        .login(&state.configuration.username, &state.configuration.password)
        .await?;
    info!("logged in");
    println!(
        "application version {}",
        client.application_version().await?
    );
    client.add_new_torrent_with_magnet(magnet).await?;

    Ok(())
}
