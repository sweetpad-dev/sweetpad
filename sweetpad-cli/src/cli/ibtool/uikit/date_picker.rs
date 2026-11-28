//! Date pickers use public setters through the same nib key/value pairs as
//! user-defined runtime attributes. This avoids depending on private picker
//! implementation views, which differ between wheel and calendar styles.
use super::controls::enumeration;
use super::{Element, Encoder, Id, Value, integer, number};

impl Encoder<'_> {
    pub(super) fn date_picker(&mut self, element: &Element, object: Id) -> Result<(), String> {
        self.control(element, object)?;
        let mode = enumeration(
            element,
            "datePickerMode",
            &[
                ("time", 0),
                ("date", 1),
                ("dateAndTime", 2),
                ("countDownTimer", 3),
            ],
            2,
        )?;
        let interval = integer(element, "minuteInterval")?.unwrap_or(1);
        if !(1..=30).contains(&interval) || 60 % interval != 0 {
            return Err("date picker minuteInterval must divide 60 and be between 1 and 30".into());
        }
        for (path, value) in [("datePickerMode", mode), ("minuteInterval", interval)] {
            let value = self
                .graph
                .shared("NSNumber", vec![("NS.intval", Value::Int(value))]);
            self.runtime_property(object, path, value);
        }
        if element.attr("style").is_some() {
            let style = enumeration(
                element,
                "style",
                &[
                    ("automatic", 0),
                    ("wheels", 1),
                    ("compact", 2),
                    ("inline", 3),
                ],
                0,
            )?;
            let value = self
                .graph
                .shared("NSNumber", vec![("NS.intval", Value::Int(style))]);
            self.runtime_property(object, "preferredDatePickerStyle", value);
        }
        if let Some(duration) = number(element, "countDownDuration")? {
            if !duration.is_finite() || !(0.0..86400.0).contains(&duration) {
                return Err("invalid date picker countDownDuration".into());
            }
            let value = self.graph.number(duration);
            self.runtime_property(object, "countDownDuration", value);
        }
        for child in &element.children {
            match child.name.as_str() {
                "locale" => {
                    if child.attr("key") != Some("locale") {
                        return Err("unsupported date picker locale key".into());
                    }
                    let name = child
                        .attr("localeIdentifier")
                        .ok_or("a locale has no identifier")?;
                    let name = self.graph.string(name);
                    let locale = self
                        .graph
                        .shared("NSLocale", vec![("NS.identifier", Value::Object(name))]);
                    self.runtime_property(object, "locale", locale);
                }
                "date" => {
                    let key = child.attr("key").ok_or("a date has no key")?;
                    if !matches!(key, "date" | "minimumDate" | "maximumDate") {
                        return Err(format!("unsupported date picker date key {key}"));
                    }
                    let time = number(child, "timeIntervalSinceReferenceDate")?
                        .filter(|n| n.is_finite())
                        .ok_or("a date has no finite reference interval")?;
                    let date = self
                        .graph
                        .shared("NSDate", vec![("NS.time", Value::Double(time))]);
                    self.runtime_property(object, key, date);
                }
                _ => {}
            }
        }
        Ok(())
    }
}
