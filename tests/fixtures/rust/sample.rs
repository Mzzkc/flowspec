use std::collections::HashMap;
use std::io::Read;

pub const MAX_SIZE: usize = 1024;
static COUNTER: u32 = 0;

pub enum Status {
    Active,
    Inactive,
}

pub trait Drawable {
    fn draw(&self);
}

pub struct Canvas {
    width: u32,
    height: u32,
}

impl Canvas {
    pub fn new(width: u32, height: u32) -> Self {
        Canvas { width, height }
    }

    fn render(&self) {
        let map = HashMap::new();
        println!("Rendering {}x{}", self.width, self.height);
    }
}

fn helper() -> i32 {
    42
}

pub fn process(data: Vec<String>) -> usize {
    let result = helper();
    data.len()
}
