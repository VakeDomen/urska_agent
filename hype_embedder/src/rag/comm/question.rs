use ollama_rs::generation::completion::request::GenerationRequest;

#[derive(Debug, Clone)]
pub struct Question {
    system_prompt: String,
    question: String,
    context: Vec<String>,
    model: String,
    body: Option<String>,
}

impl From<String> for Question {
    fn from(value: String) -> Self {
        Self {
            system_prompt: "You are a helpful assistant. Answer users question based on provided context.".to_owned(),
            question: value,
            context: vec![],
            model: "qwen3:8b".to_owned(),
            body: None,
        }
    }
}

impl From<&str> for Question {
    fn from(value: &str) -> Self {
        Self {
            system_prompt: "You are a helpful assistant. Answer users question based on provided context.".to_owned(),
            question: value.to_owned(),
            context: vec![],
            model: "qwen3:8b".to_owned(),
            body: None,
        }
    }
}

impl<'a> Into<GenerationRequest<'a>> for &'a Question {
    fn into(self) -> GenerationRequest<'a> {
        let context = if self.context.is_empty() {
            "".to_string()
        } else {
            self.context.join("\n")
        };

        let mut final_prompt = format!("{}\n{}\n{}", self.system_prompt, self.question, context);

        if self.system_prompt.contains("{{context}}") {
            final_prompt = self.system_prompt.clone();
            final_prompt = final_prompt.replace("{{context}}", &context);
        }
        GenerationRequest::new(self.model.clone(), final_prompt)
    }
}


impl Question {
    pub fn set_system_prompt(mut self, prompt: &str) -> Self {
        self.system_prompt = prompt.to_string();
        self
    }

    pub fn set_model(mut self, model: &str) -> Self {
        self.model = model.to_string();
        self
    }

    pub fn set_question(mut self, question: &str) -> Self {
        self.question = question.to_string();
        self
    }

    pub fn set_context(mut self, context: Vec<String>) -> Self {
        self.context = context;
        self
    }
}
