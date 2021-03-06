use crate::module::ModuleSpec;
use crate::value::Upvalue;
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

pub struct Agent {
    pub string_table: Vec<String>,
    pub upvalues: Vec<Rc<RefCell<Upvalue>>>,
    pub modules: HashMap<usize, ModuleSpec>,
}

impl Agent {
    pub fn new() -> Agent {
        Agent {
            string_table: Vec::new(),
            upvalues: Vec::new(),
            modules: HashMap::new(),
        }
    }

    pub fn intern_string(&mut self, s: &str) -> usize {
        if let Some(idx) = self
            .string_table
            .iter()
            .position(|ref interned| *interned == s)
        {
            idx
        } else {
            let idx = self.string_table.len();
            self.string_table.push(String::from(s));
            idx
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_string_interning() {
        let mut agent = Agent::new();

        let a = agent.intern_string("hello");
        let b = agent.intern_string("world");
        let c = agent.intern_string("hello");

        assert_eq!(a, c);
        assert_ne!(b, c);
    }
}
