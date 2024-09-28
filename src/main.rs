mod client;
mod state;
mod util;

use crate::client::{QBittorrentClient, RequestType};
use crate::state::BotState;
use crate::util::{run_in_docker, ResultExt};
use anyhow::anyhow;
use bytes::Bytes;
use config::{Config, Environment};
use dotenvy::dotenv;
use log::{info, warn, LevelFilter};
use serde::Deserialize;
use std::ops::Deref;
use telers::methods::{AnswerCallbackQuery, GetFile, SendMessage};
use telers::types::message::{Document, Text};
use telers::types::{CallbackQuery, ChatIdKind, InlineKeyboardButton, InlineKeyboardMarkup};
use telers::{
    enums::UpdateType,
    event::{telegram::HandlerResult, EventReturn, ToServiceProvider},
    types::Message,
    Bot, Dispatcher, Router,
};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    dotenv().ok();
    env_logger::builder().filter_level(LevelFilter::Info).init();

    let configuration = load_config()?;

    info!("starting application, in docker {}", run_in_docker());
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
        .register(BotState::new(configuration)?);

    router.callback_query.register(commands_callback_handler);
    router.message.register(commands_handler);
    router.message.register(torrents_handler);

    let dispatcher = Dispatcher::builder()
        .main_router(router)
        .bot(bot)
        .allowed_update(UpdateType::Message)
        .allowed_update(UpdateType::CallbackQuery)
        .build();

    Ok(dispatcher
        .to_service_provider_default()
        .unwrap()
        .run_polling()
        .await?)
}

async fn commands_callback_handler(bot: Bot, callback: CallbackQuery) -> HandlerResult {
    bot.send(AnswerCallbackQuery::new(callback.id.clone()))
        .await?;

    match &callback.data {
        Some(data) if data.as_ref() == "shutdown" => {
            bot.send(SendMessage::new(
                ChatIdKind::id(callback.chat_id().unwrap().clone()),
                "Выключение...",
            ))
                .await?;
        }
        _ => {}
    }

    Ok(EventReturn::Finish)
}

async fn commands_handler(bot: Bot, message: Message, state: BotState) -> HandlerResult {
    if let Some(from) = message.from() {
        if !state.user_allowed(from.id) {
            warn!("Unknown user id: {}", from.id);
            return Ok(EventReturn::Finish);
        }

        match message.text() {
            Some(text) if text == "/commands" => {
                bot.send(
                    SendMessage::new(message.chat().id(), "Доступные команды").reply_markup(
                        InlineKeyboardMarkup::new(vec![vec![InlineKeyboardButton::new(
                            "☠️Выключить",
                        )
                            .callback_data("shutdown")]]),
                    ),
                )
                    .await?;
                return Ok(EventReturn::Finish);
            }
            _ => return Ok(EventReturn::Skip),
        }
    }

    Ok(EventReturn::Skip)
}

async fn torrents_handler(bot: Bot, message: Message, state: BotState) -> HandlerResult {
    if let Some(from) = message.from() {
        if !state.user_allowed(from.id) {
            warn!("Unknown user id: {}", from.id);
            return Ok(EventReturn::Finish);
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
            Err(_) => Some("⛔Ошибка добавления торрента"),
        };

        if let Some(text) = text {
            bot.send(SendMessage::new(message.chat().id(), text))
                .await?;
        }
    }

    Ok(EventReturn::default())
}

enum Income {
    Enqueued,
    Skipped,
}

async fn add_torrent_by_magnet(state: &BotState, text: &Box<Text>) -> anyhow::Result<()> {
    add_new_torrent(&state, RequestType::Url(text.text.as_ref())).await?;
    Ok(())
}

async fn add_torrent_by_file(
    bot: &Bot,
    state: &BotState,
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
    url: Option<String>,
}

fn load_config() -> anyhow::Result<Configuration> {
    let config = Config::builder()
        .add_source(config::File::with_name("config"))
        .add_source(Environment::default().prefix("bittorrent_bot"))
        .build()?;

    Ok(config.try_deserialize::<Configuration>()?)
}

async fn download_torrent_file(
    state: &BotState,
    token: &str,
    file_path: &str,
) -> anyhow::Result<Bytes> {
    let url = format!("https://api.telegram.org/file/bot{}/{}", token, file_path);
    let response = state.client.get(url).send().await?;
    Ok(response.bytes().await?)
}

async fn add_new_torrent<'a>(
    state: &BotState,
    request_type: RequestType<'a>,
) -> anyhow::Result<()> {
    let client = QBittorrentClient::new(&state.options.url).await?;
    client
        .login(&state.options.username, &state.options.password)
        .await?;

    client.add_new_torrent(request_type).await?;

    Ok(())
}
