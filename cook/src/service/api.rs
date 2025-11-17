pub fn service(name: impl AsRef<str>, service_file_content: String) -> Service {
    Service::new(name.as_ref().to_string(), service_file_content)
}

pub struct Service {
    pub name: String,
    pub service_file_content: String,
}

impl Service {
    pub fn new(name: String, service_file_content: String) -> Self {
        Self {
            name,
            service_file_content,
        }
    }
}
