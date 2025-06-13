pub fn user(user: impl AsRef<str>) -> User {
    User::new(user)
}

pub struct User {
    user: String,
    pub no_login: bool,
    pub home: Option<String>,
    pub shell: Option<String>,
}

impl User {
    pub fn new(user: impl AsRef<str>) -> Self {
        User {
            user: user.as_ref().to_string(),
            no_login: false,
            home: None,
            shell: None,
        }
    }
}
