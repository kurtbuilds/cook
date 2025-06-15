use std::{any::Any, io::Write, sync::Mutex};

use crate::{Host, Rule};

pub struct State {
    global_rules: Vec<Box<dyn Rule + Send + Sync + 'static>>,
}

impl State {
    pub const fn new() -> Self {
        Self {
            global_rules: Vec::new(),
        }
    }

    pub fn rules(&self) -> &[Box<dyn Rule + Send + Sync + 'static>] {
        &self.global_rules
    }

    pub fn serialize(&self, w: impl std::io::Write) {
        let json = &mut serde_json::Serializer::new(w);
        let mut erased = <dyn erased_serde::Serializer>::erase(json);
        for rule in self.rules() {
            rule.erased_serialize(&mut erased)
                .expect("failed to serialize");
        }
    }

    pub fn add_rule(&mut self, rule: Box<dyn Rule + Send + Sync + 'static>) {
        self.global_rules.push(rule);
    }

    pub fn merge(&mut self, other: State) {
        self.global_rules.extend(other.global_rules);
    }

    pub fn hosts(&self) -> Vec<String> {
        self.global_rules
            .iter()
            .filter_map(|rule| {
                let rule = rule.as_ref();
                let rule = rule as &dyn Any;
                rule.downcast_ref::<Host>().map(|h| h.name().to_string())
            })
            // .map(|h| h.name().to_string())
            .collect::<Vec<_>>()
    }
}

static STATE: Mutex<State> = Mutex::new(State::new());

pub fn add_to_state(rule: impl Rule + Send + Sync + 'static) {
    STATE.lock().unwrap().global_rules.push(Box::new(rule));
}

pub fn drop_last_rule(identifier: &str) {
    let Some(rule) = STATE.lock().unwrap().global_rules.pop() else {
        panic!("No last rule to drop");
    };
    let id = rule.identifier();
    if id != identifier {
        panic!("Dropped rule {id}, but expected to drop rule {identifier}");
    }
}

extern "C" fn serialize_state_to_stdout() {
    let state = STATE.lock().unwrap();
    let mut stdout = std::io::stdout().lock();
    state.serialize(&mut stdout);
    stdout.write("\n".as_bytes());
}

#[cfg(feature = "atexit")]
#[ctor::ctor]
fn register_at_exit() {
    unsafe {
        let result = libc::atexit(serialize_state_to_stdout);
        if result != 0 {
            panic!("Failed to register cook serialization on atexit");
        }
    }
}
