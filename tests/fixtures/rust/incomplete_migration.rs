// Planted incomplete migration: deprecated_parse + parse_value coexist
fn deprecated_parse(input: &str) -> Option<i32> {
    input.parse::<i32>().ok()
}

fn parse_value(input: &str) -> Result<i32, String> {
    input.parse::<i32>().map_err(|e| e.to_string())
}

fn old_handler(s: &str) {
    let _ = deprecated_parse(s);
}

fn new_handler(s: &str) {
    let _ = parse_value(s);
}
