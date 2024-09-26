mod client;
mod util;

use crate::client::{QBittorrentClient, RequestType};
use crate::util::ResultExt;
use anyhow::anyhow;
use bytes::Bytes;
use config::{Config, Environment};
use dotenvy::dotenv;
use log::{info, warn, LevelFilter};
use reqwest::Client;
use serde::Deserialize;
use std::ops::Deref;
use std::sync::{Arc, Mutex};
use telers::client::Reqwest;
use telers::errors::EventErrorKind;
use telers::methods::{GetFile, SendMessage};
use telers::middlewares::outer::MiddlewareResponse;
use telers::middlewares::OuterMiddleware;
use telers::router::Request;
use telers::types::message::{Document, Text};
use telers::{
    enums::UpdateType,
    event::{telegram::HandlerResult, EventReturn, ToServiceProvider},
    types::Message,
    Bot, Dispatcher, FromContext, Router,
};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    dotenv().ok();
    env_logger::builder().filter_level(LevelFilter::Info).init();

    let configuration = load_config()?;

    info!("starting application");
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
    router.message.register(shutdown_handler);

    let dispatcher = Dispatcher::builder()
        .main_router(router)
        .bot(bot)
        .allowed_update(UpdateType::Message)
        .build();

    let shutdown_signal = Arc::new(Mutex::new(false));
    let shutdown_signal_clone = Arc::clone(&shutdown_signal);

    tokio::spawn(async move {
        while !*shutdown_signal_clone.lock().unwrap() {
            tokio::time::sleep(tokio::time::Duration::from_secs(1)).await;
        }
        info!("Shutting down the server...");
    });

    Ok(dispatcher
        .to_service_provider_default()
        .unwrap()
        .run_polling()
        .await?)
}

async fn echo_handler(bot: Bot, message: Message, state: State) -> HandlerResult {
    if let Some(from) = message.from() {
        if !state.configuration.user_id.contains(&from.id.to_string()) {
            warn!("Unknown user id: {}", from.id);
            return Ok(EventReturn::default());
        }

        let result = match &message {
            Message::Document(document) => add_torrent_by_file(&bot, &state, document)
                .await
                .map(|_| Income::Enqueued),
            Message::Text(text) => {
                if text.text.starts_with("magnet:?") {
                    add_torrent_by_magnet(&state, &text)
                        .await
                        .map(|_| Income::Enqueued)
                } else {
                    warn!("Unexpected text message received: {}", text.text);
                    Ok(Income::Skipped)
                }
            }
            _ => Ok(Income::Skipped),
        }
        .log_error();

        let text = match result {
            Ok(income) => match income {
                Income::Enqueued => Some("✅Торрент добавлен в очередь"),
                Income::Skipped => None,
            },
            Err(_) => Some("�Ошибка добавления торрента"),
        };

        if let Some(text) = text {
            bot.send(SendMessage::new(message.chat().id(), text))
                .await?;
        }
    }

    Ok(EventReturn::default())
}

async fn shutdown_handler(bot: Bot, message: Message, state: State) -> HandlerResult {
    if let Some(from) = message.from() {
        if !state.configuration.user_id.contains(&from.id.to_string()) {
            warn!("Unknown user id: {}", from.id);
            return Ok(EventReturn::default());
        }

        if let Message::Text(text) = &message {
            if text.text == state.configuration.shutdown_command {
                *state.shutdown_signal.lock().unwrap() = true;
                bot.send(SendMessage::new(message.chat().id(), "Server is shutting down..."))
                    .await?;
            }
        }
    }

    Ok(EventReturn::default())
}

enum Income {
    Enqueued,
    Skipped,
}

async fn add_torrent_by_magnet(state: &State, text: &Box<Text>) -> anyhow::Result<()> {
    add_new_torrent(&state, RequestType::Url(text.text.as_ref())).await?;
    Ok(())
}

async fn add_torrent_by_file(
    bot: &Bot,
    state: &State,
    document: &Box<Document>,
) -> anyhow::Result<()> {
    let file_id = &document.document.file_id;
    let file_info = bot.send(GetFile::new(file_id.deref())).await?;
    let file_path = file_info
        .file_path
        .ok_or(anyhow!("File path not available after fet file info"))?;
    let file = download_torrent_file(&state, &bot.token, &file_path).await?;
    add_new_torrent(&state, RequestType::File(&Bytes::from(file))).await?;
    Ok(())
}

#[derive(Debug, Deserialize, Clone)]
struct Configuration {
    bot_token: String,
    user_id: String,
    username: String,
    password: String,
    url: String,
    shutdown_command: String,
}

#[derive(Clone, FromContext)]
#[context(key = "state")]
struct State {
    inner: Arc<Inner>,
    shutdown_signal: Arc<Mutex<bool>>,
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
            shutdown_signal: Arc::new(Mutex::new(false)),
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
    Ok(response.bytes().await?)
}

async fn add_new_torrent<'a>(state: &State, request_type: RequestType<'a>) -> anyhow::Result<()> {
    let client = QBittorrentClient::new(&state.configuration.url).await?;
    client
        .login(&state.configuration.username, &state.configuration.password)
        .await?;

    client.add_new_torrent(request_type).await?;

    Ok(())
}
