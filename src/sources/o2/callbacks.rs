use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct NameValue {
    pub name: String,
    pub value: Value,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Callback {
    #[serde(rename = "type")]
    pub callback_type: String,
    #[serde(default)]
    pub output: Vec<NameValue>,
    #[serde(default)]
    pub input: Vec<NameValue>,
    #[serde(rename = "_id")]
    pub id: u32,
}

impl Callback {
    pub fn output_value(&self, name: &str) -> Option<&Value> {
        self.output
            .iter()
            .find(|item| item.name == name)
            .map(|item| &item.value)
    }

    pub fn hidden_id(&self) -> Option<&str> {
        self.output_value("id").and_then(Value::as_str)
    }

    pub fn set_input_value(&mut self, value: Value) {
        if let Some(first) = self.input.first_mut() {
            first.value = value;
        }
    }
}

#[derive(Deserialize)]
#[serde(untagged)]
pub enum AuthChainResponse {
    Final(FinalAuth),
    InProgress(InProgressAuth),
}

#[derive(Deserialize)]
pub struct FinalAuth {
    #[serde(rename = "tokenId")]
    pub token_id: String,
}

#[derive(Serialize, Deserialize, Clone)]
pub struct InProgressAuth {
    #[serde(rename = "authId")]
    pub auth_id: String,
    pub callbacks: Vec<Callback>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stage: Option<String>,
}

pub fn find_by_id<'a>(callbacks: &'a mut [Callback], id: &str) -> Option<&'a mut Callback> {
    callbacks
        .iter_mut()
        .find(|callback| callback.hidden_id() == Some(id))
}

pub fn find_by_type<'a>(
    callbacks: &'a mut [Callback],
    callback_type: &str,
) -> Option<&'a mut Callback> {
    callbacks
        .iter_mut()
        .find(|callback| callback.callback_type == callback_type)
}

pub fn find_pow_script(callbacks: &[Callback]) -> Option<String> {
    for callback in callbacks {
        if callback.callback_type != "TextOutputCallback" {
            continue;
        }

        let Some(message) = callback.output_value("message").and_then(Value::as_str) else {
            continue;
        };

        if message.contains("startProofOfWork") {
            return Some(message.to_string());
        }
    }

    None
}

pub fn default_choice(callback: &Callback) -> i64 {
    callback
        .output_value("defaultChoice")
        .and_then(Value::as_i64)
        .unwrap_or(0)
}

pub fn confirmation_option_index(callback: &Callback, name: &str) -> Option<i64> {
    callback
        .output_value("options")?
        .as_array()?
        .iter()
        .position(|option| option.as_str() == Some(name))
        .map(|index| index as i64)
}
