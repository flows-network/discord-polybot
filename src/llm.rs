use anyhow;
use async_openai::{
    config::Config as OpenAIConfig,
    types::{
        ChatCompletionRequestMessage,
        // ChatCompletionFunctionsArgs, ChatCompletionRequestMessage,
        ChatCompletionRequestSystemMessageArgs,
        ChatCompletionRequestUserMessageArgs,
        // ChatCompletionTool, ChatCompletionToolArgs, ChatCompletionToolType,
        CreateChatCompletionRequestArgs,
        // FinishReason,
    },
    Client as OpenAIClient,
};
use reqwest::header::HeaderMap;
use secrecy::Secret;
use store::Expire;
use std::{ collections::HashMap, vec };
use std::env;
use store_flows as store;

pub async fn chat_rounds_n(
    client: OpenAIClient<LocalServiceProviderConfig>,
    messages: Vec<ChatCompletionRequestMessage>,
    max_token: u16,
    model: &str
) -> anyhow::Result<String> {
    let request = CreateChatCompletionRequestArgs::default()
        .max_tokens(max_token)
        .model(model)
        .messages(messages.clone())
        .build()?;

    match client.chat().create(request).await {
        Ok(chat) =>
            match chat.choices[0].message.clone().content {
                Some(res) => {
                    log::info!("{:?}", res.clone());
                    Ok(res)
                }
                None => Err(anyhow::anyhow!("Failed to get reply from OpenAI")),
            }
        Err(_e) => {
            log::error!("Error getting response from hosted LLM: {:?}", _e);
            Err(anyhow::Error::new(_e))
        }
    }
}

pub async fn chat_inner_async(
    system_prompt: &str,
    user_input: &str,
    max_token: u16,
    model: &str
) -> anyhow::Result<String> {
    use reqwest::header::{ HeaderValue, CONTENT_TYPE, USER_AGENT };
    let api_key = env::var("LLM_API_KEY").expect("LLM_API_KEY-must-be-set");
    let api_base = env::var("LLM_API_BASE").unwrap_or(String::from("http://52.37.228.1:8080/v1"));
    let mut headers = HeaderMap::new();
    headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
    headers.insert(USER_AGENT, HeaderValue::from_static("MyClient/1.0.0"));
    let config = LocalServiceProviderConfig {
        // api_base: String::from("http://127.0.0.1:8080/v1"),
        api_base: api_base,
        headers,
        api_key: Secret::new(api_key),
        query: HashMap::new(),
    };

    let client = OpenAIClient::with_config(config);
    let messages = vec![
        ChatCompletionRequestSystemMessageArgs::default()
            .content(system_prompt)
            .build()
            .expect("Failed to build system message")
            .into(),
        ChatCompletionRequestUserMessageArgs::default().content(user_input).build()?.into()
    ];
    let request = CreateChatCompletionRequestArgs::default()
        .max_tokens(max_token)
        .model(model)
        .messages(messages)
        .build()?;

    match client.chat().create(request).await {
        Ok(chat) =>
            match chat.choices[0].message.clone().content {
                Some(res) => {
                    log::info!("{:?}", res.clone());
                    Ok(res)
                }
                None => Err(anyhow::anyhow!("Failed to get reply from OpenAI")),
            }
        Err(_e) => {
            log::error!("Error getting response from hosted LLM: {:?}", _e);
            Err(anyhow::anyhow!(_e))
        }
    }
}

#[derive(Clone, Debug)]
pub struct LocalServiceProviderConfig {
    pub api_base: String,
    pub headers: HeaderMap,
    pub api_key: Secret<String>,
    pub query: HashMap<String, String>,
}

impl OpenAIConfig for LocalServiceProviderConfig {
    fn headers(&self) -> HeaderMap {
        self.headers.clone()
    }

    fn url(&self, path: &str) -> String {
        format!("{}{}", self.api_base, path)
    }

    fn query(&self) -> Vec<(&str, &str)> {
        self.query
            .iter()
            .map(|(k, v)| (k.as_str(), v.as_str()))
            .collect()
    }

    fn api_base(&self) -> &str {
        &self.api_base
    }

    fn api_key(&self) -> &Secret<String> {
        &self.api_key
    }
}

pub fn create_llm_client() -> OpenAIClient<LocalServiceProviderConfig> {
    use reqwest::header::{ HeaderValue, CONTENT_TYPE, USER_AGENT };
    let api_key = env::var("LLM_API_KEY").expect("LLM_API_KEY-must-be-set");
    let api_base = env::var("LLM_API_BASE").unwrap_or(String::from("http://52.37.228.1:8080/v1"));
    let mut headers = HeaderMap::new();
    headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
    headers.insert(USER_AGENT, HeaderValue::from_static("MyClient/1.0.0"));
    let config = LocalServiceProviderConfig {
        // api_base: String::from("http://127.0.0.1:8080/v1"),
        api_base: api_base,
        headers,
        api_key: Secret::new(api_key),
        query: HashMap::new(),
    };
    OpenAIClient::with_config(config)
}

pub fn chat_history(current_q: &str, restart: bool) -> Vec<String> {
    let mut chat_history: Vec<String> = if restart {
        vec![current_q.to_string()]
    } else {
        store
            ::get("chat_history")
            .and_then(|v|
                v.as_array().map(|arr| {
                    arr.iter()
                        .filter_map(|val| val.as_str().map(String::from))
                        .collect()
                })
            )
            .unwrap_or_else(Vec::new)
    };

    if !restart {
        chat_history.push(current_q.to_string());
        if chat_history.len() > 8 {
            chat_history.remove(0);
        }
    }

    store::set(
        "chat_history",
        serde_json::json!(chat_history),
        Some(Expire {
            kind: store::ExpireKind::Ex,
            value: 300,
        })
    );

    chat_history
}

/* pub async fn context_manager(current_q: &str) -> anyhow::Result<Vec<String>> {
    let mut chat_history: Vec<String> = if restart {
        vec![current_q.to_string()]
    } else {
        store
            ::get("chat_history")
            .and_then(|v|
                v.as_array().map(|arr| {
                    arr.iter()
                        .filter_map(|val| val.as_str().map(String::from))
                        .collect()
                })
            )
            .unwrap_or_else(Vec::new)
    };
    let system_prompt = format!(
        r#"You are the context manager AI of a less capable team member. Your teammate engages in a multi-turn conversation with user, the user may switch topics any time, but it lacks the power to sense such changes, you task is to examine its original task, its chat history to judge whether the user has actually started a new topic, please gauage this propability and reply in JSON format, please use "Y" to indicate the probability of a topic change greater than 50%, "N" to indicate otherwise, this is its task: {bot_prompt}, following is the {chat_history}:
    {{
    \"topic_changed\": \"Y\"
    }}
    Ensure that the JSON is properly formatted, with correct escaping of special characters, and is ready to be parsed by a JSON parser that expects RFC8259-compliant JSON. Avoid adding any non-JSON content or formatting."#
    );

    if parse_topic_change_from_json(current_q).await {
        let current_q = chat_history.pop().unwrap_or_else(|| "".to_string());

        chat_history = vec![current_q.to_string()];
    } else {
        log::info!("Topic not changed");
    }
    chat_history
} */

pub async fn parse_topic_change_from_json(input: &str) -> bool {
    use regex::Regex;
    use serde_json::{ Value, from_str };
    use log;

    let parsed_result: Result<Value, serde_json::Error> = from_str(input);

    match parsed_result {
        Ok(parsed) => {
            if let Some(Value::String(value)) = parsed.get("topic_changed") {
                match value.as_str() {
                    "Y" => {
                        return true;
                    }
                    "N" => {
                        return false;
                    }
                    _ => log::error!("Unexpected value for 'topic_changed'"),
                }
            } else {
                log::error!("'topic_changed' key not found or not a string");
            }
        }
        Err(e) => {
            log::error!("Error parsing JSON: {:?}", e);

            let re = Regex::new(r#""topic_changed":\s*"([YN])""#).expect(
                "Failed to compile regex pattern"
            );

            if let Some(cap) = re.captures(input) {
                match cap.get(1).map(|m| m.as_str()) {
                    Some("Y") => {
                        return true;
                    }
                    Some("N") => {
                        return false;
                    }
                    _ => log::error!("No valid 'topic_changed' value found"),
                }
            }
        }
    }
    false
}
