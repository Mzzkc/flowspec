pub struct Engine {
    running: bool,
}

impl Engine {
    pub fn start(&mut self) {
        self.initialize();
        self.running = true;
    }

    fn initialize(&self) {
        // setup
    }
}

fn main() {
    let mut e = Engine { running: false };
    e.start();
}
