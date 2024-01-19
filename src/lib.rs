pub mod llm;
use llm::chat_inner_async;
use base64::{ engine::general_purpose, Engine };
use cloud_vision_flows::text_detection;
use discord_flows::{
    application_command_handler,
    http::{ Http, HttpBuilder },
    message_handler,
    model::{
        application::interaction::InteractionResponseType,
        prelude::application::interaction::application_command::ApplicationCommandInteraction,
        Attachment,
        Message,
    },
    Bot,
    ProvidedBot,
};
use dotenv::dotenv;
use flowsnet_platform_sdk::logger;
use once_cell::sync::Lazy;
use serde_json::{ json, Value };
use std::collections::HashMap;
use std::{ env, str };
use store::Expire;
use store_flows as store;
use web_scraper_flows::get_page_text;

static PROMPTS: Lazy<HashMap<&'static str, Value>> = Lazy::new(|| {
    let mut map = HashMap::new();
    map.insert("start", json!("You are a helpful assistant answering questions on Discord."));
    map.insert(
        "summarize",
        json!(
            "You are a helpful assistant trained to summarize text in short bullet points. Please always answer in English even if the original text is not in English. Be prepared that you might be asked questions related to the content you summarize."
        )
    );
    map.insert(
        "code",
        json!(
            "You are an experienced software developer trained to review computer source code, explain what it does, identify potential problems, and suggest improvements. Please always answer in English. Be prepared that you might be asked follow-up questions related to the source code."
        )
    );
    map.insert(
        "medical",
        json!(
            "You are a medical doctor trained to read and summarize lab reports. The text you receive will contain medical lab results. Please analyze them and present the major findings as short bullet points, followed by a one-sentence summary about the subject's health status. All answers should be in English. Be prepared to answer follow-up questions related to the lab report."
        )
    );
    map.insert(
        "translate",
        json!(
            "You are an English language translator. For every message you receive, please translate it to English. Please respond with just the English translation and nothing more. If the input message is already in English, please fix any grammar errors and improve the writing."
        )
    );
    map.insert(
        "reply_tweet",
        json!(
            "You are a social media marketing expert. You will receive the text from a tweet. Please generate 3 clever replies to it. Then follow user suggestions to improve the reply tweets."
        )
    );
    map
});

#[no_mangle]
#[tokio::main(flavor = "current_thread")]
pub async fn on_deploy() {
    dotenv().ok();
    logger::init();
    let discord_token = env::var("discord_token").unwrap();
    let bot_id = env::var("bot_id").unwrap();

    let bot = ProvidedBot::new(&discord_token);

    _ = register_commands(&discord_token, &bot_id).await;
    bot.listen_to_messages().await;
    bot.listen_to_application_commands().await;
}

#[message_handler]
async fn handle(msg: Message) {
    let discord_token = env::var("discord_token").unwrap();
    let bot_id = env::var("bot_id").unwrap();

    let bot = ProvidedBot::new(&discord_token);
    let client = bot.get_client();

    if msg.author.bot {
        log::info!("ignored bot message");
        return;
    }

    if msg.member.is_some() {
        let mut mentions_me = false;
        for u in &msg.mentions {
            log::debug!("The user ID is {}", u.id.as_u64());
            if *u.id.to_string() == bot_id {
                mentions_me = true;
                break;
            }
        }
        if !mentions_me {
            log::debug!("ignored guild message");
            return;
        }
    }

    if msg.attachments.len() == 0 && msg.content.trim().is_empty() {
        return;
    }

    if let Some((key, system_prompt, restart)) = prompt_checking() {
        let mut question = String::new();

        match msg.attachments.len() {
            0 => {
                let clean_input = if msg.content.starts_with("@") {
                    msg.content
                        .chars()
                        .skip_while(|&c| c != ' ')
                        .skip(1)
                        .collect::<String>()
                } else {
                    msg.content.to_owned()
                };
                question = clean_input.clone();
                if key.as_str() == "summarize" {
                    let possible_url = clean_input.trim().to_string();

                    if let Ok(u) = http_req::uri::Uri::try_from(possible_url.as_str()) {
                        match get_page_text(&possible_url).await {
                            Ok(text) => {
                                question = if text.len() > 36_000 {
                                    text.chars().take(36_000).collect::<String>()
                                } else {
                                    text.clone()
                                };
                            }
                            Err(_e) => {}
                        };
                    }
                }
            }
            _ => {
                question = process_attachments(&msg, &client).await;
            }
        }

        if let Some(res) = process_input(&system_prompt, &question, restart).await {
            let resps = sub_strings(&res, 1800);
            let content = &format!("Answer: {} ", resps[0]);
            _ = client.send_message(
                msg.channel_id.into(),
                &json!({
                      "content": content
                    })
            ).await;

            if resps.len() > 1 {
                for resp in resps.iter().skip(1) {
                    let content = &format!("Answer: {}", resp);
                    _ = client.send_message(
                        msg.channel_id.into(),
                        &json!({
                              "content": content
                            })
                    ).await;
                }
            }
        }
    }
}

#[application_command_handler]
async fn handler(ac: ApplicationCommandInteraction) {
    let token = env::var("discord_token").unwrap();

    let _bot = ProvidedBot::new(&token);
    let client = _bot.get_client();
    client.set_application_id(ac.application_id.into());
    handle_command(client, ac).await;
    return;
}

async fn handle_command(client: Http, ac: ApplicationCommandInteraction) {
    _ = client.create_interaction_response(
        ac.id.into(),
        &ac.token,
        &json!({"type": InteractionResponseType::DeferredChannelMessageWithSource as u8}
            )
    ).await;

    let mut msg = "";
    let help_msg = env
        ::var("help_msg")
        .unwrap_or(
            "You can enter text or upload an image with text to chat with this bot. The bot can take several different assistant roles. Type command /qa or /translate or /summarize or /medical or /code or /reply_tweet to start.".to_string()
        );

    match ac.data.name.as_str() {
        "help" => {
            msg = &help_msg;
        }
        "start" => {
            set_current_prompt_key("start");
            msg = &help_msg;
        }
        "summarize" => {
            set_current_prompt_key("summarize");
            msg = "I'm ready to summarize, please input a url or text";
        }
        "code" => {
            set_current_prompt_key("code");
            msg = "I'm ready to review source code";
        }
        "medical" => {
            set_current_prompt_key("medical");
            msg = "I am ready to review and summarize doctor notes or medical test results";
        }
        "translate" => {
            set_current_prompt_key("translate");
            msg = "I'm ready to translate";
        }
        "reply_tweet" => {
            set_current_prompt_key("reply_tweet");
            msg = "I'm ready to process your tweet";
        }
        "qa" => {
            set_current_prompt_key("qa");
            msg = "I'm ready for your questions";
        }
        _ => {}
    }
    _ = client.edit_original_interaction_response(
        &ac.token,
        &json!(
                { "content": msg }
            )
    ).await;
    store::set(
        "previous_prompt_key",
        json!(String::new()),
        Some(Expire {
            kind: store::ExpireKind::Ex,
            value: 1,
        })
    );

    return;
}

async fn process_attachments(msg: &Message, client: &Http) -> String {
    let attachments = get_attachments((*msg.attachments).to_vec());
    let mut question = String::new();
    let mut writer = Vec::new();

    for (url, is_txt) in attachments {
        log::debug!("Try to DOWNLOAD {}", &url);
        if is_txt {
            match http_req::request::get(url.clone(), &mut writer) {
                Ok(res) => {
                    if res.status_code().is_success() {
                        let content = String::from_utf8_lossy(&writer);
                        question.push_str(&content);
                    }
                }
                Err(_) => {
                    log::warn!("Could not download text from {}", &url);
                    continue;
                }
            };
        } else {
            let bs64 = match download_image(url) {
                Ok(b) => b,
                Err(e) => {
                    log::warn!("{}", e);
                    _ = client.send_message(
                        msg.channel_id.into(),
                        &json!({
                            "content": "There is a problem with the uploaded file. Can you try again?"
                        })
                    ).await;
                    continue;
                }
            };
            log::debug!("Downloaded size {}", bs64.len());
            let detected = match text_detection(bs64) {
                Ok(t) => {
                    log::debug!("text_detection: {}", t);
                    t
                }
                Err(e) => {
                    log::debug!("The input image does not contain text: {}", e);
                    continue;
                }
            };
            question.push_str(&detected);
        }
        question.push_str("\n");
    }
    return question;
}

fn get_attachments(attachments: Vec<Attachment>) -> Vec<(String, bool)> {
    let mut typ = String::new();
    let res = attachments
        .iter()
        .filter_map(|a| {
            typ = a.content_type.as_deref().unwrap_or("no file type").to_string();
            if let Some(ct) = a.content_type.as_ref() {
                if ct.starts_with("image") {
                    return Some((a.url.clone(), false));
                }
                if ct.starts_with("text") {
                    return Some((a.url.clone(), true));
                }
            }
            None
        })
        .collect();

    log::error!("{:?}", typ);
    log::info!("{:?}", typ);
    return res;
}

fn download_image(url: String) -> Result<String, String> {
    let mut writer = Vec::new();
    let resp = http_req::request::get(url, &mut writer);

    match resp {
        Ok(r) => {
            if r.status_code().is_success() {
                Ok(general_purpose::STANDARD.encode(writer))
            } else {
                Err(
                    format!(
                        "response failed: {}, body: {}",
                        r.reason(),
                        String::from_utf8_lossy(&writer)
                    )
                )
            }
        }
        Err(e) => Err(e.to_string()),
    }
}

async fn process_input(system_prompt: &str, question: &str, restart: bool) -> Option<String> {

    // let co = ChatOptions {
    //     // model: ChatModel::GPT4,
    //     model: ChatModel::GPT35Turbo16K,
    //     restart: restart,
    //     system_prompt: Some(system_prompt),
    //     ..Default::default()
    // };

    match chat_inner_async(&system_prompt, &question, 512, "llama2-chat-7b").await {
        Ok(r) => Some(r),
        Err(e) => {
            log::error!("OpenAI returns error: {}", e);
            None
        }
    }
}

fn sub_strings(string: &str, sub_len: usize) -> Vec<&str> {
    let mut subs = Vec::with_capacity(string.len() / sub_len);
    let mut iter = string.chars();
    let mut pos = 0;

    while pos < string.len() {
        let mut len = 0;
        for ch in iter.by_ref().take(sub_len) {
            len += ch.len_utf8();
        }
        subs.push(&string[pos..pos + len]);
        pos += len;
    }
    subs
}

pub async fn register_commands(discord_token: &str, bot_id: &str) -> bool {
    let commands =
        json!([
        {
            "name": "help",
            "description": "Display help message"
        },
        {
            "name": "start",
            "description": "Start a conversation with the assistant"
        },
        {
            "name": "summarize",
            "description": "Generate a summary on given url",

        },
        {
            "name": "code",
            "description": "Review source code"
        },
        {
            "name": "medical",
            "description": "Review and summarize doctor notes or medical test results"
        },
        {
            "name": "translate",
            "description": "Translate anything into English",

        },
        {
            "name": "reply_tweet",
            "description": "Reply a tweet for you",
        },
        {
            "name": "qa",
            "description": "I'm ready for general QA",

        }
    ]);

    let http_client = HttpBuilder::new(discord_token)
        .application_id(bot_id.parse().unwrap())
        .build();

    match http_client.create_global_application_commands(&commands).await {
        Ok(_) => {
            log::info!("Successfully registered command");
            true
        }
        Err(err) => {
            log::error!("Error registering command: {}", err);
            false
        }
    }
}

pub fn set_previous_prompt_key(key: &str) {
    store::set(
        "previous_prompt_key",
        json!(key.to_string()),
        Some(Expire {
            kind: store::ExpireKind::Ex,
            value: 300,
        })
    );
}

pub fn set_current_prompt_key(key: &str) {
    store::set(
        "current_prompt_key",
        json!(key.to_string()),
        Some(Expire {
            kind: store::ExpireKind::Ex,
            value: 60,
        })
    );
}

pub fn prompt_checking() -> Option<(String, String, bool)> {
    let current_prompt_key = store
        ::get("current_prompt_key")
        .and_then(|v| v.as_str().map(String::from));
    let previous_prompt_key = store
        ::get("previous_prompt_key")
        .and_then(|v| v.as_str().map(String::from));
    let mut restart = true;
    let system_prompt;
    let prompt_key;
    match (current_prompt_key.as_deref(), previous_prompt_key.as_deref()) {
        (Some(cur), may_exist) => {
            if let Some(prv) = may_exist {
                if cur == prv {
                    restart = false;
                }
            }
            prompt_key = cur.to_string();
            system_prompt = match PROMPTS.get(cur).unwrap_or(&Value::Null).as_str() {
                Some(str) => str.to_string(),
                None => String::new(),
            };
            set_previous_prompt_key(cur);
        }
        (None, Some(prv)) => {
            restart = false;
            system_prompt = match PROMPTS.get(prv).unwrap_or(&Value::Null).as_str() {
                Some(str) => str.to_string(),
                None => String::new(),
            };
            prompt_key = prv.to_string();

            set_previous_prompt_key(prv);
        }
        (None, None) => {
            return None;
        }
    }

    Some((prompt_key, system_prompt, restart))
}
