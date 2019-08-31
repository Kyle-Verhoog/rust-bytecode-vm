use std::cell::RefCell;
use std::collections::HashMap;
use std::convert::{TryFrom, TryInto};
use std::ops::{Add, Div, Mul, Rem, Sub};
use std::rc::Rc;

use crate::agent::Agent;
use crate::bytecode::Bytecode;
use crate::code_object::CodeObject;
use crate::opcode::OpCode;
use crate::value::{FunctionValue, Upvalue, Value};

macro_rules! print_stack {
    ($stack:expr) => {{
        print!("[");
        for (i, v) in $stack.iter().enumerate() {
            print!("{:?}", v);
            if i < $stack.len() - 1 {
                print!(", ");
            }
        }
        println!("]");
    }};
}

pub struct Interpreter<'a> {
    agent: &'a mut Agent<'a>,
    global: HashMap<usize, Value>,
    call_stack: Vec<usize>,
    stack: Vec<Value>,
    ip: usize,
    bp: usize,
    sp: usize,
}

impl<'a> Interpreter<'a> {
    pub fn new(agent: &'a mut Agent<'a>) -> Interpreter<'a> {
        Interpreter::with_global(agent, HashMap::new())
    }

    pub fn with_global(agent: &'a mut Agent<'a>, global: HashMap<usize, Value>) -> Interpreter<'a> {
        Interpreter {
            agent,
            global,
            call_stack: Vec::new(),
            stack: Vec::new(),
            ip: 0,
            bp: 0,
            sp: 0,
        }
    }

    pub fn evaluate(&mut self, code_object: CodeObject) -> Result<Value, String> {
        macro_rules! push {
            ($expr:expr) => {{
                self.sp += 1;
                self.stack.push($expr)
            }};
        }
        macro_rules! pop {
            () => {{
                let val = self.stack.pop().ok_or("Stack underflow")?;
                self.sp -= 1;
                val
            }};
            ($num:expr) => {{
                for _ in 0..$num {
                    pop!();
                }
            }};
        }
        macro_rules! next {
            () => {{
                let inst = code_object.instructions.get(self.ip);
                self.ip += 1;
                inst
            }};
            ($expr:expr) => {{
                let mut array = [0u8; $expr];

                for i in 0..$expr {
                    let result: Result<&u8, String> =
                        next!().ok_or("Unexpected end of bytecode".into());
                    let n: u8 = *result?;
                    array[i] = n;
                }

                array
            }};
        }

        macro_rules! number_binop {
            ($name:expr, $intop:expr, $doubleop:expr) => {
                number_binop!($name, $intop, $doubleop, |a: i64| -> Result<i64, String> {
                    Ok(a)
                })
            };
            ($name:expr, $intop:expr, $doubleop:expr, $bconvert:expr) => {{
                let b = pop!();
                let a = pop!();

                push!(if let Value::Integer(a) = a {
                    if let Value::Integer(b) = b {
                        Value::from($intop(a, ($bconvert)(b)?))
                    } else if let Value::Double(b) = b {
                        Value::from($doubleop(a as f64, b))
                    } else {
                        panic!("Got unexpected value {:?} in {}", b, $name);
                    }
                } else if let Value::Double(a) = a {
                    if let Value::Integer(b) = b {
                        Value::from($doubleop(a, b as f64))
                    } else if let Value::Double(b) = b {
                        Value::from($doubleop(a, b))
                    } else {
                        panic!("Got unexpected value {:?} in {}", b, $name);
                    }
                } else {
                    panic!("Got unexpected value {:?} in {}", a, $name);
                })
            }};
        }

        while let Some(instruction) = next!() {
            if cfg!(vm_debug) {
                println!("--------------");
                print_stack!(&self.stack);
                println!("{:?}", OpCode::try_from(instruction)?);
                println!("ip: {} sp: {} bp: {}", self.ip, self.sp, self.bp);
            }

            match OpCode::try_from(instruction)? {
                OpCode::Halt => break,

                OpCode::ConstInt => {
                    push!(Value::from(i64::from_le_bytes(next!(8))));
                }

                OpCode::ConstDouble => {
                    push!(Value::from(f64::from_bits(u64::from_le_bytes(next!(8))),));
                }

                OpCode::ConstNull => {
                    push!(Value::Null);
                }

                OpCode::ConstTrue => {
                    push!(Value::from(true));
                }

                OpCode::ConstFalse => {
                    push!(Value::from(false));
                }

                OpCode::ConstString => {
                    let idx = usize::from_le_bytes(next!(8));
                    push!(Value::from(self.agent.string_table[idx]));
                }

                OpCode::Add => number_binop!("addition", i64::add, f64::add),
                OpCode::Sub => number_binop!("subtraction", i64::sub, f64::sub),
                OpCode::Mul => number_binop!("multiplication", i64::mul, f64::mul),
                OpCode::Div => number_binop!("division", i64::div, f64::div),
                OpCode::Mod => number_binop!("modulus", i64::rem, f64::rem),
                OpCode::Exp => number_binop!(
                    "exponentiation",
                    i64::pow,
                    f64::powf,
                    |b: i64| -> Result<u32, String> {
                        b.try_into().map_err(|_| "Integer overflow".to_string())
                    }
                ),

                OpCode::Jump => {
                    self.ip = usize::from_le_bytes(next!(8));
                }

                OpCode::JumpIfTrue => {
                    let to = usize::from_le_bytes(next!(8));
                    let cond = pop!();
                    if cond.is_truthy() {
                        self.ip = to;
                    }
                }

                OpCode::JumpIfFalse => {
                    let to = usize::from_le_bytes(next!(8));
                    let cond = pop!();
                    if !cond.is_truthy() {
                        self.ip = to;
                    }
                }

                OpCode::Call => {
                    let function = pop!();
                    let num_args = usize::from_le_bytes(next!(8));
                    if let Value::Function(f) = &function {
                        macro_rules! ensure_arity {
                            ($arity:expr, $name:expr) => {{
                                if num_args < $arity {
                                    let name = if let Some(name) = $name {
                                        self.agent.string_table[*name]
                                    } else {
                                        "<anonymous>"
                                    };
                                    return Err(format!(
                                        "Function {} expected {} args, got {}",
                                        name, $arity, num_args
                                    ));
                                }
                            }};
                        }
                        match f {
                            FunctionValue::Builtin {
                                arity,
                                function,
                                name,
                                ..
                            } => {
                                ensure_arity!(*arity, name);
                                let mut args = Vec::new();
                                for _ in 0..num_args {
                                    args.push(pop!());
                                }
                                let result = function(self, args);
                                push!(result);
                            }
                            FunctionValue::User {
                                arity,
                                address,
                                name,
                                ..
                            } => {
                                ensure_arity!(*arity, name);
                                self.call_stack.push(self.ip); // return address
                                self.call_stack.push(num_args); // for cleanup
                                self.call_stack.push(self.bp); // current base pointer
                                self.bp = self.sp; // new base is at current stack index
                                self.ip = *address; // jump into function
                                push!(function);
                            }
                        }
                    } else {
                        return Err(format!("Value {:?} is not callable", function));
                    }
                }

                OpCode::Return => {
                    let retval = pop!();
                    self.bp = self.call_stack.pop().ok_or("Missing bp on call_stack")?;
                    let num_args = self
                        .call_stack
                        .pop()
                        .ok_or("Missing num_args on call_stack")?;
                    self.ip = self.call_stack.pop().ok_or("Missing ip on call_stack")?;

                    while let Some(uv) = self.agent.upvalues.pop() {
                        if uv.borrow().is_open() {
                            let i = uv.borrow().stack_index();
                            if i < self.sp - num_args {
                                self.agent.upvalues.push(uv);
                                break;
                            }
                            uv.borrow_mut().close(self.stack[i].clone());
                        } else {
                            return Err("Had closed upvalue in agent.upvalues".to_string());
                        }
                    }

                    pop!(num_args);
                    push!(retval);
                }

                OpCode::Pop => {
                    pop!();
                }

                OpCode::LoadLocal => {
                    push!(self.stack[self.bp + usize::from_le_bytes(next!(8))].clone());
                }

                OpCode::StoreLocal => {
                    self.stack[self.bp + usize::from_le_bytes(next!(8))] =
                        self.stack[self.sp - 1].clone();
                }

                OpCode::LoadGlobal => {
                    let id = usize::from_le_bytes(next!(8));
                    if let Some(val) = self.global.get(&id) {
                        push!(val.clone());
                    } else {
                        return Err(format!(
                            "ReferenceError: {} is not defined",
                            self.agent.string_table[id]
                        ));
                    }
                }

                OpCode::StoreGlobal => {
                    let id = usize::from_le_bytes(next!(8));
                    if self.global.contains_key(&id) {
                        self.global.insert(id, pop!());
                    } else {
                        return Err(format!(
                            "ReferenceError: {} is not defined",
                            self.agent.string_table[id]
                        ));
                    }
                }

                OpCode::NewFunction => {
                    let arity = usize::from_le_bytes(next!(8));
                    let address = usize::from_le_bytes(next!(8));

                    push!(Value::from(FunctionValue::User {
                        name: None,
                        address,
                        arity,
                        upvalues: Vec::new(),
                    }));
                }

                OpCode::BindLocal => {
                    let idx = usize::from_le_bytes(next!(8));
                    let mut func = pop!();

                    if let Value::Function(FunctionValue::User { upvalues, .. }) = &mut func {
                        let upvalue = Rc::new(RefCell::new(Upvalue::new(idx)));
                        self.agent.upvalues.push(upvalue.clone());
                        upvalues.push(upvalue);
                    } else {
                        return Err("Cannot bind local to non-user function".to_string());
                    }
                }

                OpCode::BindUpvalue => {
                    let idx = usize::from_le_bytes(next!(8));
                    let mut func = pop!();

                    if let Value::Function(FunctionValue::User { upvalues, .. }) = &mut func {
                        let uv =
                            self.agent.upvalues.iter().find(|uv| {
                                uv.borrow().is_open() && uv.borrow().stack_index() == idx
                            });
                        if let Some(upvalue) = uv {
                            upvalues.push(upvalue.clone());
                        }
                    }
                }
            }
        }

        Ok(if let Some(value) = self.stack.pop() {
            value
        } else {
            Value::Null
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    #[test]
    fn test_halt() {
        let mut agent = Agent::new();
        let mut interpreter = Interpreter::new(&mut agent);

        let bytecode = Bytecode::new().halt().const_true().into();

        let result = interpreter.evaluate(CodeObject::new(bytecode));
        assert_eq!(result, Ok(Value::Null));
    }

    #[test]
    fn test_const_int() {
        let mut agent = Agent::new();
        let mut interpreter = Interpreter::new(&mut agent);

        let bytecode = Bytecode::new().const_int(123).into();

        let code = CodeObject::new(bytecode);
        let result = interpreter.evaluate(code);

        assert_eq!(result, Ok(Value::from(123)));
    }

    #[test]
    fn test_const_double() {
        let mut agent = Agent::new();
        let mut interpreter = Interpreter::new(&mut agent);

        let bytecode = Bytecode::new().const_double(1.23).into();

        let code = CodeObject::new(bytecode);

        let result = interpreter.evaluate(code);
        assert_eq!(result, Ok(Value::from(1.23)));
    }

    #[test]
    fn test_const_true() {
        let mut agent = Agent::new();
        let mut interpreter = Interpreter::new(&mut agent);

        let bytecode = Bytecode::new().const_true().into();

        let result = interpreter.evaluate(CodeObject::new(bytecode));
        assert_eq!(result, Ok(Value::from(true)));
    }

    #[test]
    fn test_const_false() {
        let mut agent = Agent::new();
        let mut interpreter = Interpreter::new(&mut agent);

        let bytecode = Bytecode::new().const_false().into();

        let result = interpreter.evaluate(CodeObject::new(bytecode));
        assert_eq!(result, Ok(Value::from(false)));
    }

    #[test]
    fn test_const_null() {
        let mut agent = Agent::new();
        let mut interpreter = Interpreter::new(&mut agent);

        let bytecode = Bytecode::new().const_null().into();

        let result = interpreter.evaluate(CodeObject::new(bytecode));
        assert_eq!(result, Ok(Value::Null));
    }

    #[test]
    fn test_const_string() {
        let mut agent = Agent::new();

        let bytecode = bytecode! {
            const_string (agent) "hello world"
        };

        let mut interpreter = Interpreter::new(&mut agent);
        let code = CodeObject::new(bytecode.into());

        let result = interpreter.evaluate(code);
        assert_eq!(result, Ok(Value::from("hello world")));
    }

    #[test]
    fn test_add() {
        let mut agent = Agent::new();
        let mut interpreter = Interpreter::new(&mut agent);

        let bytecode = bytecode! {
            const_int 123
            const_double 1.23
            add
        };

        let code = CodeObject::new(bytecode.into());
        let result = interpreter.evaluate(code);

        assert_eq!(result, Ok(Value::from(124.23)));
    }

    #[test]
    fn test_sub() {
        let mut agent = Agent::new();
        let mut interpreter = Interpreter::new(&mut agent);

        let bytecode = bytecode! {
            const_int 123
            const_double 1.23
            sub
        };

        let code = CodeObject::new(bytecode.into());
        let result = interpreter.evaluate(code);

        assert_eq!(result, Ok(Value::from(121.77)));
    }

    #[test]
    fn test_mul() {
        let mut agent = Agent::new();
        let mut interpreter = Interpreter::new(&mut agent);

        let bytecode = bytecode! {
            const_int 123
            const_double 2.0
            mul
        };

        let code = CodeObject::new(bytecode.into());
        let result = interpreter.evaluate(code);

        assert_eq!(result, Ok(Value::from(246f64)));
    }

    #[test]
    fn test_div() {
        let mut agent = Agent::new();
        let mut interpreter = Interpreter::new(&mut agent);

        let bytecode = bytecode! {
            const_int 124
            const_double 2.0
            div
        };

        let code = CodeObject::new(bytecode.into());
        let result = interpreter.evaluate(code);

        assert_eq!(result, Ok(Value::from(62f64)));
    }

    #[test]
    fn test_mod() {
        let mut agent = Agent::new();
        let mut interpreter = Interpreter::new(&mut agent);

        let bytecode = bytecode! {
            const_int 124
            const_double 2.0
            mod
        };

        let code = CodeObject::new(bytecode.into());
        let result = interpreter.evaluate(code);

        assert_eq!(result, Ok(Value::from(0f64)));
    }

    #[test]
    fn test_exp() {
        let mut agent = Agent::new();
        let mut interpreter = Interpreter::new(&mut agent);

        let bytecode = bytecode! {
            const_int 4
            const_int 2
            exp
        };

        let code = CodeObject::new(bytecode.into());
        let result = interpreter.evaluate(code);

        assert_eq!(result, Ok(Value::from(16)));
    }

    #[test]
    fn test_jump() {
        let mut agent = Agent::new();
        let mut interpreter = Interpreter::new(&mut agent);

        let bytecode = bytecode! {
            const_int 4
            jump 29
            const_int 8
            add
            halt
            const_int 12
            mul
            halt
        };

        let code = CodeObject::new(bytecode.into());
        let result = interpreter.evaluate(code);

        assert_eq!(result, Ok(Value::from(48)));
    }

    #[test]
    fn test_jump_if_true() {
        let mut agent = Agent::new();

        let bytecode = bytecode! {
            const_int 123
            const_int 234
            const_int 1
            jump_if_true one
            mul
            halt
        one:
            const_false
            jump_if_true two
            add
            halt
        two:
            sub
        };

        let code = CodeObject::new(bytecode.into());
        let mut interpreter = Interpreter::new(&mut agent);
        let result = interpreter.evaluate(code);

        assert_eq!(result, Ok(Value::from(357)));
    }

    #[test]
    fn test_jump_if_false() {
        let mut agent = Agent::new();

        let bytecode = bytecode! {
            const_true
            jump_if_false one
            const_int 10
            const_int 2
            const_string (agent) ""
            jump_if_false two
        one:
            add
            halt
        two:
            mul
            halt
        };

        let code = CodeObject::new(bytecode.into());
        let mut interpreter = Interpreter::new(&mut agent);
        let result = interpreter.evaluate(code);

        assert_eq!(result, Ok(Value::from(20)));
    }

    #[test]
    fn test_user_function() {
        let mut agent = Agent::new();

        let name = agent.intern_string("ret123");
        let ret123 = Value::from(FunctionValue::User {
            name: Some(name),
            address: 9,
            arity: 0,
            upvalues: Vec::new(),
        });

        let mut global = HashMap::new();
        global.insert(name, ret123);

        let bytecode = bytecode! {
            jump main

            const_int 123
            return

        main:
            load_global name
            call 0
        };

        let code = CodeObject::new(bytecode.into());
        crate::disassemble::disassemble(&agent, &code).unwrap();
        let mut interpreter = Interpreter::with_global(&mut agent, global);
        let result = interpreter.evaluate(code);

        assert_eq!(result, Ok(Value::from(123)));
    }

    #[test]
    fn test_pop() {
        let mut agent = Agent::new();

        let bytecode = bytecode! {
            const_int 123
            pop
        };

        let code = CodeObject::new(bytecode.into());
        let mut interpreter = Interpreter::new(&mut agent);
        let result = interpreter.evaluate(code);

        assert_eq!(result, Ok(Value::Null));
    }

    #[test]
    fn test_load_local() {
        let mut agent = Agent::new();

        let bytecode = bytecode! {
            const_int 123
            const_double 432.0

            load_local 0
        };

        let code = CodeObject::new(bytecode.into());
        let mut interpreter = Interpreter::new(&mut agent);
        let result = interpreter.evaluate(code);

        assert_eq!(result, Ok(Value::from(123)));
    }

    #[test]
    fn test_store_local() {
        let mut agent = Agent::new();

        let bytecode = bytecode! {
            const_int 123

            const_int 234
            store_local 0
            pop

            load_local 0
        };

        let code = CodeObject::new(bytecode.into());
        let mut interpreter = Interpreter::new(&mut agent);
        let result = interpreter.evaluate(code);

        assert_eq!(result, Ok(Value::from(234)));
    }

    #[test]
    fn test_load_global() {
        let mut agent = Agent::new();
        let mut global = HashMap::new();

        global.insert(agent.intern_string("test"), Value::from("test"));

        let bytecode = bytecode! {
            load_global (agent.intern_string("test"))
        };

        let code = CodeObject::new(bytecode.into());
        let mut interpreter = Interpreter::with_global(&mut agent, global);
        let result = interpreter.evaluate(code);

        assert_eq!(result, Ok(Value::from("test")));
    }

    #[test]
    fn test_store_global() {
        let mut agent = Agent::new();
        let mut global = HashMap::new();

        global.insert(agent.intern_string("test"), Value::from(3));

        let bytecode = bytecode! {
            load_global (agent.intern_string("test"))
            const_int 3
            exp
        };

        let code = CodeObject::new(bytecode.into());
        let mut interpreter = Interpreter::with_global(&mut agent, global);
        let result = interpreter.evaluate(code);

        assert_eq!(result, Ok(Value::from(27)));
    }

    #[test]
    fn test_new_function() {
        let mut agent = Agent::new();

        let bytecode = bytecode! {
            jump main

        func:
            const_int 999
            return

        main:
            new_function 0 func
            call 0
        };

        let code = CodeObject::new(bytecode.into());
        let mut interpreter = Interpreter::new(&mut agent);
        let result = interpreter.evaluate(code);

        assert_eq!(result, Ok(Value::from(999)));
    }
}
