use async_openai::{
    config::Config,
    types::{
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
use std::collections::HashMap;
use std::env;
use anyhow;

pub async fn chat_inner_async(
    system_prompt: &str,
    user_input: &str,
    max_token: u16,
    model: &str
) -> anyhow::Result<String> {
    use reqwest::header::{ HeaderValue, CONTENT_TYPE, USER_AGENT };
    let token = env::var("DEEP_API_KEY").unwrap_or(String::from("DEEP_API_KEY-must-be-set"));
    let mut headers = HeaderMap::new();
    headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
    headers.insert(USER_AGENT, HeaderValue::from_static("MyClient/1.0.0"));
    let config = LocalServiceProviderConfig {
        // api_base: String::from("http://127.0.0.1:8080/v1"),
        api_base: String::from("http://52.37.228.1:8080/v1"),
        headers: headers,
        api_key: Secret::new(token),
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

impl Config for LocalServiceProviderConfig {
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
