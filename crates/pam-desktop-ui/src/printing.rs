//! Bounded text documents in typographic points, rendered by native print systems.
use serde_json::{Map, Value};

pub type PrintCompletion = Box<dyn FnOnce(Result<bool, String>) + Send>;

#[derive(Clone, Debug)]
pub struct Document {
    pub title: String,
    pub width: f64,
    pub height: f64,
    pub pages: Vec<Vec<TextBlock>>,
}

#[derive(Clone, Debug)]
pub struct TextBlock {
    pub text: String,
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
    pub size: f64,
    pub bold: bool,
    /// 1 left, 2 center, 3 right.
    pub align: u8,
}

impl Document {
    pub fn decode(json: &str) -> Result<Self, String> {
        if json.len() > 2 * 1024 * 1024 {
            return Err("Print document exceeds two megabytes".into());
        }
        let value: Value = serde_json::from_str(json)
            .map_err(|error| format!("Invalid print document: {error}"))?;
        let map = object(&value, &["title", "width", "height", "pages"])?;
        let source_pages = map
            .get("pages")
            .and_then(Value::as_array)
            .ok_or("Missing print pages")?;
        if source_pages.len() > 100 {
            return Err("Too many print pages".into());
        }
        let mut pages = Vec::with_capacity(source_pages.len());
        for source_page in source_pages {
            let source_blocks = source_page.as_array().ok_or("Invalid print page")?;
            if source_blocks.len() > 5000 {
                return Err("Too many print blocks".into());
            }
            let mut blocks = Vec::with_capacity(source_blocks.len());
            for source in source_blocks {
                let block = object(
                    source,
                    &["text", "x", "y", "width", "height", "size", "bold", "align"],
                )?;
                let align = block
                    .get("align")
                    .and_then(Value::as_u64)
                    .filter(|value| (1..=3).contains(value))
                    .ok_or("Invalid print alignment")? as u8;
                blocks.push(TextBlock {
                    text: text(block, "text")?,
                    x: number(block, "x")?,
                    y: number(block, "y")?,
                    width: number(block, "width")?,
                    height: number(block, "height")?,
                    size: number(block, "size")?,
                    bold: block
                        .get("bold")
                        .and_then(Value::as_bool)
                        .ok_or("Missing print weight")?,
                    align,
                });
            }
            pages.push(blocks);
        }
        let document = Self {
            title: text(map, "title")?,
            width: number(map, "width")?,
            height: number(map, "height")?,
            pages,
        };
        if document.title.is_empty() || document.title.len() > 256 || document.title.contains('\0')
        {
            return Err("Print title must contain 1–256 bytes".into());
        }
        if !bounded(document.width, 72.0, 2880.0)
            || !bounded(document.height, 72.0, 8640.0)
            || document.pages.is_empty()
            || document.pages.len() > 100
        {
            return Err("Invalid print paper size or page count".into());
        }
        let mut blocks = 0;
        let mut bytes = 0;
        for page in &document.pages {
            blocks += page.len();
            for block in page {
                bytes += block.text.len();
                if block.text.contains('\0')
                    || !bounded(block.x, 0.0, document.width)
                    || !bounded(block.y, 0.0, document.height)
                    || !bounded(block.width, 1.0, document.width)
                    || !bounded(block.height, 1.0, document.height)
                    || !bounded(block.size, 4.0, 144.0)
                    || block.x + block.width > document.width + 0.001
                    || block.y + block.height > document.height + 0.001
                    || !(1..=3).contains(&block.align)
                {
                    return Err("Invalid print text block".into());
                }
            }
        }
        if blocks > 5000 || bytes > 1024 * 1024 {
            return Err("Print document exceeds its block/text budget".into());
        }
        Ok(document)
    }
}
fn object<'a>(value: &'a Value, allowed: &[&str]) -> Result<&'a Map<String, Value>, String> {
    let map = value.as_object().ok_or("Expected print object")?;
    if map.keys().any(|key| !allowed.contains(&key.as_str())) {
        return Err("Unknown print document field".into());
    }
    Ok(map)
}
fn text(map: &Map<String, Value>, key: &str) -> Result<String, String> {
    map.get(key)
        .and_then(Value::as_str)
        .map(str::to_owned)
        .ok_or_else(|| format!("Missing print text {key}"))
}
fn number(map: &Map<String, Value>, key: &str) -> Result<f64, String> {
    map.get(key)
        .and_then(Value::as_f64)
        .ok_or_else(|| format!("Missing print coordinate {key}"))
}
fn bounded(value: f64, low: f64, high: f64) -> bool {
    value.is_finite() && value >= low && value <= high
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn print_documents_reject_invalid_coordinates_unknown_fields_and_unbounded_work() {
        let valid = r#"{"title":"Receipt","width":226.77,"height":400,"pages":[[{"text":"Café R$ 19,90","x":12,"y":12,"width":200,"height":20,"size":10,"bold":false,"align":1}]]}"#;
        assert!(Document::decode(valid).is_ok());
        assert!(Document::decode(&valid.replace("\"align\":1", "\"align\":0")).is_err());
        assert!(Document::decode(&valid.replace("\"width\":200", "\"width\":1000")).is_err());
        assert!(Document::decode(&valid.replace("\"x\":12", "\"x\":-1")).is_err());
        assert!(
            Document::decode(&valid.replace("\"bold\":false", "\"bold\":false,\"script\":true"))
                .is_err()
        );
        assert!(
            Document::decode(r#"{"title":"Receipt","width":226,"height":400,"pages":[]}"#).is_err()
        );
    }
}
