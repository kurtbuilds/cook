pub fn package(name: impl AsRef<str>) -> Package {
    Package::new(name.as_ref().to_string())
}

pub struct Package {
    name: String,
}

impl Package {
    pub fn new(name: String) -> Self {
        Self { name }
    }
}
