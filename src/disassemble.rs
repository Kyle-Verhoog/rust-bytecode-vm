use std::convert::TryFrom;

use crate::agent::Agent;
use crate::code_object::CodeObject;
use crate::opcode::OpCode;

pub fn disassemble(agent: &Agent, code_object: &CodeObject) -> Result<(), String> {
    let mut ip = 0;

    macro_rules! next {
        () => {{
            let inst = code_object.instructions.get(ip);
            ip += 1;
            inst
        }};
        ($count:expr) => {{
            let mut arr = [0u8; $count];

            for i in 0..$count {
                let result = next!().ok_or("Unexpected end of bytecode".to_string());
                arr[i] = *result?;
            }

            arr
        }};
    }

    while let Some(instruction) = next!() {
        match OpCode::try_from(instruction)? {
            instruction @ OpCode::ConstInt => {
                println!("{:?}({:?})", instruction, i64::from_le_bytes(next!(8)));
            }

            instruction @ OpCode::ConstDouble => {
                println!(
                    "{:?}({:?})",
                    instruction,
                    f64::from_bits(u64::from_le_bytes(next!(8)))
                );
            }

            instruction @ OpCode::ConstString => {
                println!(
                    "{:?}({:?})",
                    instruction,
                    agent.string_table[usize::from_le_bytes(next!(8))]
                );
            }

            instruction => println!("{:?}", instruction),
        }
    }

    Ok(())
}