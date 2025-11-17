pub fn which(bin: impl AsRef<str>, script: impl AsRef<str>) -> Which {
    Which {
        bin: bin.as_ref().to_string(),
        script: Some(script.as_ref().to_string()),
    }
}

pub struct Which {
    pub bin: String,
    pub script: Option<String>,
}

impl Which {
    pub fn new(bin: impl AsRef<str>) -> Self {
        Which {
            bin: bin.as_ref().to_string(),
            script: None,
        }
    }
}
