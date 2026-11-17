//! A world's own calendar ("自定义历法"): how years are shown — an era's
//! name, which year is its first, and what years before it are called.
//! Years inside the document stay plain integers; only their display
//! changes, so files from before calendars and every stored year are
//! untouched, and turning the calendar off shows plain years again.

use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Calendar {
    /// Shown instead of plain years.
    pub on: bool,
    /// The era's name, e.g. "新历".
    pub era: String,
    /// The (plain) year that is the era's year 1.
    pub epoch: i32,
    /// Put before years ahead of the era, e.g. "前".
    pub before: String,
    /// What a year looks like: `{era}` and `{n}` are replaced, e.g. "{era}{n}年".
    pub pattern: String,
}

impl Default for Calendar {
    fn default() -> Self {
        Calendar { on: false, era: "新历".into(), epoch: 1, before: "前".into(), pattern: "{era}{n}年".into() }
    }
}

impl Calendar {
    /// The year as shown ("1914 年" when the calendar is off).
    pub fn show(&self, y: i32) -> String {
        if !self.on {
            return format!("{y} 年");
        }
        let (n, era) = if y >= self.epoch {
            ((y as i64 - self.epoch as i64 + 1).to_string(), self.era.clone())
        } else {
            // the year before the first is year 1 before the era (no year 0)
            ((self.epoch as i64 - y as i64).to_string(), format!("{}{}", self.era, self.before))
        };
        self.pattern.replace("{era}", &era).replace("{n}", &n)
    }

    /// The year as shown, without the "年" a plain year carries (for titles).
    pub fn show_short(&self, y: i32) -> String {
        if self.on {
            self.show(y)
        } else {
            y.to_string()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn years_in_an_era() {
        let mut c = Calendar::default();
        assert_eq!(c.show(1914), "1914 年");
        assert_eq!(c.show_short(1914), "1914");
        c.on = true;
        c.epoch = 1700;
        assert_eq!(c.show(1700), "新历1年");
        assert_eq!(c.show(1914), "新历215年");
        assert_eq!(c.show(1699), "新历前1年", "no year 0");
        assert_eq!(c.show(1600), "新历前100年");
        c.pattern = "{era} {n}".into();
        c.era = "AR".into();
        c.before = " before".into();
        assert_eq!(c.show(1690), "AR before 10");
        // older files: no calendar, plain years
        let back: Calendar = serde_json::from_str("{}").unwrap();
        assert_eq!(back, Calendar::default());
    }
}
