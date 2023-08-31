use base64::{engine::general_purpose, Engine};
use cloud_vision_flows::text_detection;
use discord_flows::{
    application_command_handler,
    http::{Http, HttpBuilder},
    message_handler,
    model::{
        application::interaction::InteractionResponseType,
        application_command::CommandDataOptionValue,
        prelude::application::interaction::application_command::ApplicationCommandInteraction,
        Attachment, Message,
    },
    Bot, ProvidedBot,
};
use dotenv::dotenv;
use flowsnet_platform_sdk::logger;
use once_cell::sync::Lazy;
use openai_flows::{
    chat::{ChatModel, ChatOptions},
    OpenAIFlows,
};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::{env, str};
use store::Expire;
use store_flows as store;
use web_scraper_flows::get_page_text;

static PROMPTS: Lazy<HashMap<&'static str, Value>> = Lazy::new(|| {
    let mut map = HashMap::new();
    map.insert(
        "start",
        json!("You are a helpful assistant answering questions on Discord."),
    );
    map.insert("summarize", json!("You are a helpful assistant trained to summarize text in short bullet points. Please always answer in English even if the original text is not in English. Be prepared that you might be asked questions related to the content you summarize."
));
    map.insert("code", json!("You are an experienced software developer trained to review computer source code, explain what it does, identify potential problems, and suggest improvements. Please always answer in English. Be prepared that you might be asked follow-up questions related to the source code."
));
    map.insert("medical", json!("You are a medical doctor trained to read and summarize lab reports. The text you receive will contain medical lab results. Please analyze them and present the major findings as short bullet points, followed by a one-sentence summary about the subject's health status. All answers should be in English. Be prepared to answer follow-up questions related to the lab report."
));
    map.insert("translate", json!("You are an English language translator. For every message you receive, please translate it to English. Please respond with just the English translation and nothing more. If the input message is already in English, please fix any grammar errors and improve the writing."));
    map.insert("reply_tweet", json!("You are a social media marketing expert. You will receive the text from a tweet. Please generate 3 clever replies to it. Then follow user suggestions to improve the reply tweets."));
    map
});

#[no_mangle]
#[tokio::main(flavor = "current_thread")]
pub async fn on_deploy() {
    dotenv().ok();
    logger::init();
    let discord_token = env::var("discord_token").unwrap();
    let channel_id = env::var("discord_channel_id").unwrap_or("channel_id not found".to_string());

    let bot = ProvidedBot::new(&discord_token);
    let commands_registered = env::var("COMMANDS_REGISTERED").unwrap_or("false".to_string());

    match commands_registered.as_str() {
        "false" => {
            register_commands(&discord_token).await;
            env::set_var("COMMANDS_REGISTERED", "true");
        }
        _ => {}
    }

    bot.listen_to_messages().await;

    let channel_id = channel_id.parse::<u64>().unwrap();
    bot.listen_to_application_commands_from_channel(channel_id)
        .await;
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

    let mut msg_question = msg.content.to_string();
    if msg.member.is_some() {
        let mut mentions_me = false;
        for u in &msg.mentions {
            log::debug!("The user ID is {}", u.id.as_u64());
            if *u.id.to_string() == bot_id {
                mentions_me = true;
                msg_question = msg_question
                    .chars()
                    .skip_while(|&c| c != ' ')
                    .skip(1) // Skip the space itself
                    .collect::<String>();

                msg_question = if msg_question.len() < 100 {
                    format!("Here is an input from the user: '{}'. If this is a question related to the previous task or source content, please provide the answer. If it is not a question, process it as new content.", msg_question)
                } else {
                    msg_question
                };
                break;
            }
        }
        if !mentions_me {
            log::debug!("ignored guild message");
            return;
        }
    }

    let ocr = if msg.attachments.len() > 0 {
        let res = process_attachments(&msg, &client).await;
        slack_flows::send_message_to_channel("ik8", "ch_err", res.to_string()).await;
        res
    } else {
        String::new()
    };

    let memory = match store::get("bot_memory") {
        Some(m) => m.as_str().unwrap_or("").to_string(),
        None => String::new(),
    };

    if let Some((key, system_prompt, restart)) = prompt_checking() {
        let question = if (msg_question.is_empty() && memory.is_empty()) {
            format!("Here is original user input: {:?}.", ocr)
        } else {
            format!(
                "Here is original user input: {:?}\n{:?}. {:?}",
                memory, ocr, msg_question
            )
        };

        set_bot_memory(&format!("{}\n{}", memory, ocr.clone()));

        // slack_flows::send_message_to_channel(
        //     "ik8",
        //     "ch_err",
        //     format!("question/mem now: {} ", question.clone()),
        // )
        // .await;

        if let Some(res) = process_input(&system_prompt, &question, restart).await {
            let resps = sub_strings(&res, 1800);
            let content = &format!("Answer: {} ", resps[0]);
            _ = client
                .send_message(
                    msg.channel_id.into(),
                    &json!({
                      "content": content
                    }),
                )
                .await;

            if resps.len() > 1 {
                for resp in resps.iter().skip(1) {
                    let content = &format!("Answer: {}", resp);
                    _ = client
                        .send_message(
                            msg.channel_id.into(),
                            &json!({
                              "content": content
                            }),
                        )
                        .await;
                }
            }
        }
    }
}

#[application_command_handler]
async fn handler(ac: ApplicationCommandInteraction) {
    let github_token = env::var("github_token").unwrap_or("fake-token".to_string());
    let token = env::var("discord_token").unwrap();
    let channel_id = env::var("discord_channel_id").unwrap_or("channel_id not found".to_string());

    let channel_id = channel_id.parse::<u64>().unwrap();
    let _bot = ProvidedBot::new(&token);
    let client = _bot.get_client();
    client.set_application_id(ac.application_id.into());
    handle_command(client, ac).await;
    return;
}

async fn handle_command(client: Http, ac: ApplicationCommandInteraction) {
    _ = client
        .create_interaction_response(
            ac.id.into(),
            &ac.token,
            &(json!(
                {
                    "type": InteractionResponseType::DeferredChannelMessageWithSource as u8,

                }
            )),
        )
        .await;

    let ac_token_head = ac.token.chars().take(8).collect::<String>();
    let options = &ac.data.options;
    let mut current_prompt_key = "";
    let mut global_text_carrier = String::new();
    let mut msg = String::new();
    let mut no_input = false;
    let help_msg = env::var("help_msg").unwrap_or("You can enter text or upload an image with text to chat with this bot. The bot can take several different assistant roles. Type command /qa or /translate or /summarize or /medical or /code or /reply_tweet to start.".to_string());

    match ac.data.name.as_str() {
        "help" => {
            _ = client
                .edit_original_interaction_response(
                    &ac.token,
                    &(json!(
                        { "content": &help_msg }
                    )),
                )
                .await;
            return;
        }
        "start" => {
            current_prompt_key = "start";
            msg = help_msg.clone();
        }
        "summarize" => {
            current_prompt_key = "summarize";
            set_current_prompt_key("summarize");

            let url = options.get(0).and_then(|opt| {
                opt.resolved.as_ref().and_then(|val| match val {
                    CommandDataOptionValue::String(s) => Some(s.as_str()),
                    _ => None,
                })
            });

            match url {
                Some(u) => match get_page_text(&u).await {
                    Ok(text) => {
                        global_text_carrier = text.clone();

                        set_bot_memory(&text);

                        msg = format!("Summarizing the text from {}", u);
                    }
                    Err(_e) => {
                        _ = client
                                .edit_original_interaction_response(
                                    &ac.token,
                                    &(json!(
                                        {"content": "You may have input an incorrect url or the Bot failed to get content from the url, please put the url or the texts to summarize in your next message"}
                                    )),
                                )
                                .await;
                        return;
                    }
                },
                None => no_input = true,
            };
            if no_input {
                _ = client
            .edit_original_interaction_response(
                &ac.token,
                &(json!(
                    {"content": "You didn't input any url for me to fetch, please input text in your next message"}
                )),
            )
            .await;
                return;
            };
        }
        "code" => {
            set_current_prompt_key("code");

            _ = client
                .edit_original_interaction_response(
                    &ac.token,
                    &(json!(
                        {"content": "I'm ready to review source code"}
                    )),
                )
                .await;
            return;
        }
        "medical" => {
            set_current_prompt_key("medical");

            _ = client
                    .edit_original_interaction_response(
                        &ac.token,
                        &(json!(
                            { "content": "I am ready to review and summarize doctor notes or medical test results" }
                        )),
                    )
                    .await;

            return;
        }
        "translate" => {
            current_prompt_key = "translate";
            set_current_prompt_key("translate");
            match options.get(0).and_then(|opt| opt.resolved.as_ref()) {
                Some(CommandDataOptionValue::String(s)) => {
                    global_text_carrier = s.as_str().to_string();

                    msg = "Translating ...".to_string();
                }
                _ => no_input = true,
            };
            if no_input {
                _ = client
            .edit_original_interaction_response(
                &ac.token,
                &(json!(
                    {"content": "You didn't input any text to translate, please input text in your next message"}
                )),
            )
            .await;
                return;
            };
        }
        "reply_tweet" => {
            current_prompt_key = "reply_tweet";
            set_current_prompt_key("reply_tweet");
            match options.get(0).and_then(|opt| opt.resolved.as_ref()) {
                Some(CommandDataOptionValue::String(s)) => {
                    global_text_carrier = s.as_str().to_string();

                    msg = "Working on the reply".to_string();
                }
                _ => no_input = true,
            };
            if no_input {
                _ = client
            .edit_original_interaction_response(
                &ac.token,
                &(json!(
                    {"content": "You didn't input any tweet for me to work with, please input it in your next message"}
                )),
            )
            .await;
                return;
            };
        }
        "qa" => {
            current_prompt_key = "qa";
            set_current_prompt_key("qa");
            match options.get(0).and_then(|opt| opt.resolved.as_ref()) {
                Some(CommandDataOptionValue::String(s)) => {
                    global_text_carrier = s.as_str().to_string();

                    msg = "Working on the answer for you".to_string();
                }
                _ => no_input = true,
            };
            if no_input {
                _ = client
            .edit_original_interaction_response(
                &ac.token,
                &(json!(
                    {"content": "I didn't see any text to reply to, please input your question in your next message"}
                )),
            )
            .await;
                return;
            };
        }
        _ => {}
    }
    _ = client
        .edit_original_interaction_response(
            &ac.token,
            &(json!(
                { "content": &msg }
            )),
        )
        .await;
    store::set(
        "previous_prompt_key",
        json!(String::new()),
        Some(Expire {
            kind: store::ExpireKind::Ex,
            value: 0,
        }),
    );

    if global_text_carrier.is_empty() || current_prompt_key.is_empty() {
        return;
    }
    set_current_prompt_key(current_prompt_key);

    let system_prompt = PROMPTS
        .get(current_prompt_key)
        .unwrap_or(&Value::Null)
        .as_str()
        .unwrap_or_default()
        .to_string();

    if let Some(res) = process_input(&system_prompt, &global_text_carrier, true).await {
        let resps = sub_strings(&res, 1800);

        _ = client
            .edit_original_interaction_response(
                &ac.token,
                &serde_json::json!({
                    "content": resps[0]
                }),
            )
            .await;
        if resps.len() > 1 {
            for resp in resps.iter().skip(1) {
                let content = &format!("Question: {}", resp);
                _ = client
                    .create_interaction_response(
                        ac.id.into(),
                        &ac.token,
                        &json!(                {
                            "type": 4,
                            "data": {
                                "content": content
                            }
                        }),
                    )
                    .await;
            }
        }
    }
    global_text_carrier.clear();

    return;
}

async fn process_attachments(msg: &Message, client: &Http) -> String {
    let attachments = get_attachments((*msg.attachments).to_vec());
    slack_flows::send_message_to_channel("ik8", "general", format!("{:?}", attachments.clone()))
        .await;

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
                    _ = client
                    .send_message(
                        msg.channel_id.into(),
                        &json!({
                            "content": "There is a problem with the uploaded file. Can you try again?"
                        }),
                    )
                    .await;
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
                    // _ = client
                    // .send_message(
                    //     msg.channel_id.into(),
                    //     &json!({
                    //         "content": "Sorry, the input image does not contain text. Can you try again"
                    //     }),
                    // )
                    // .await;
                    continue;
                }
            };
            slack_flows::send_message_to_channel("ik8", "ch_err", detected.to_string()).await;

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
            typ = a.content_type
                .as_deref()
                .unwrap_or("no file type")
                .to_string();
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
                Err(format!(
                    "response failed: {}, body: {}",
                    r.reason(),
                    String::from_utf8_lossy(&writer)
                ))
            }
        }
        Err(e) => Err(e.to_string()),
    }
}

async fn process_input(system_prompt: &str, question: &str, restart: bool) -> Option<String> {
    let mut openai = OpenAIFlows::new();
    openai.set_retry_times(2);

    if restart {
        set_bot_memory(&question);
    }
    let co = ChatOptions {
        // model: ChatModel::GPT4,
        model: ChatModel::GPT35Turbo16K,
        restart: restart,
        system_prompt: Some(system_prompt),
        ..Default::default()
    };

    match openai
        .chat_completion(&format!("bot-generation"), &question, &co)
        .await
    {
        Ok(r) => Some(r.choice),
        Err(e) => {
            log::error!("OpenAI returns error: {}", e);
            None
        }
    }
}

fn get_image_urls(attachments: Vec<Attachment>) -> Vec<String> {
    attachments
        .iter()
        .filter_map(|a| match a.content_type.as_ref() {
            Some(ct) if ct.starts_with("image") => Some(a.url.clone()),
            _ => None,
        })
        .collect()
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

pub async fn register_commands(discord_token: &str) -> bool {
    let bot_id = env::var("bot_id").unwrap_or("1140749575309758514".to_string());
    let guild_id = env::var("discord_server").unwrap_or("1126690101288775740".to_string());

    let commands = json!([
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
            "options": [
                {
                    "name": "url",
                    "description": "The url to get text and summarize on",
                    "type": 3,
                    "required": false
                }
            ]
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
            "options": [
                {
                    "name": "text",
                    "description": "Text you want to translate",
                    "type": 3,
                    "required": false
                }
            ]
        },
        {
            "name": "reply_tweet",
            "description": "Reply a tweet for you",
            "options": [
                {
                    "name": "tweet_text",
                    "description": "The tweet you want a reply for",
                    "type": 3,
                    "required": false
                }
            ]
        },
        {
            "name": "qa",
            "description": "Ready for general QA",
            "options": [
                {
                    "name": "question",
                    "description": "Your question for QA",
                    "type": 3,
                    "required": false
                }
            ]
        }
    ]);

    let guild_id = guild_id.parse::<u64>().unwrap_or(1128056245765558364);
    let http_client = HttpBuilder::new(discord_token)
        .application_id(bot_id.parse().unwrap())
        .build();

    match http_client
        .create_guild_application_commands(guild_id, &commands)
        .await
    {
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

pub fn set_bot_memory(text: &str) {
    store::set(
        "bot_memory",
        json!(text),
        Some(Expire {
            kind: store::ExpireKind::Ex,
            value: 300,
        }),
    );
}

pub fn set_previous_prompt_key(key: &str) {
    store::set(
        "previous_prompt_key",
        json!(key.to_string()),
        Some(Expire {
            kind: store::ExpireKind::Ex,
            value: 60,
        }),
    );
}

pub fn set_current_prompt_key(key: &str) {
    store::set(
        "current_prompt_key",
        json!(key.to_string()),
        Some(Expire {
            kind: store::ExpireKind::Ex,
            value: 60,
        }),
    );
}

pub fn prompt_checking() -> Option<(String, String, bool)> {
    let current_prompt_key =
        store::get("current_prompt_key").and_then(|v| v.as_str().map(String::from));
    let previous_prompt_key =
        store::get("previous_prompt_key").and_then(|v| v.as_str().map(String::from));
    let mut restart = true;
    let mut system_prompt = String::new();
    let mut prompt_key = String::new();
    match (
        current_prompt_key.as_deref(),
        previous_prompt_key.as_deref(),
    ) {
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
        (None, None) => return None,
    }

    Some((prompt_key, system_prompt, restart))
}
