pub fn ghrelease(name: impl AsRef<str>) -> GhRelease {
    GhRelease::new(name.as_ref().to_string())
}

pub struct GhRelease {
    name: String,
}

impl GhRelease {
    pub fn new(name: String) -> Self {
        Self { name }
    }
}
