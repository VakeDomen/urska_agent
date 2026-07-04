use std::collections::HashMap;
use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
pub enum ProfileRole {
    Student,
    Employee,
    Admin,
}

#[derive(Debug, Clone, Serialize)]
pub struct StudentInfo {
    pub faculty_code: String,
    pub enrolment_year: u16,
    pub study_level: u8,
    pub seq_number: u16,
}

#[derive(Debug, Clone, Serialize)]
pub struct Profile {
    pub role: ProfileRole,
    pub username: String,
    pub raw_attributes: HashMap<String, Vec<String>>,
}

impl Profile {
    pub fn try_from_student_string(ldap_resp: String, attrs: HashMap<String, Vec<String>>) -> Result<Self, String> {
        let username = extract_uid(&ldap_resp)?;
        Ok(Self { role: ProfileRole::Student, username, raw_attributes: attrs })
    }

    pub fn try_from_employee_string(ldap_resp: String, attrs: HashMap<String, Vec<String>>) -> Result<Self, String> {
        let username = extract_uid(&ldap_resp)?;
        Ok(Self { role: ProfileRole::Employee, username, raw_attributes: attrs })
    }

    /// Returns a clean, LLM-friendly context with only meaningful fields.
    pub fn sanitized_context(&self) -> serde_json::Value {
        let get = |key: &str| self.raw_attributes.get(key).and_then(|v| v.first()).map(|s| s.as_str());

        let mut ctx = serde_json::Map::new();
        ctx.insert("role".into(), serde_json::json!(self.role));
        ctx.insert("username".into(), serde_json::json!(self.username));
        ctx.insert("name".into(), serde_json::json!(get("cn").or_else(|| get("displayName")).unwrap_or("")));
        ctx.insert("email".into(), serde_json::json!(get("mail").unwrap_or("")));
        ctx.insert("country".into(), serde_json::json!(get("schacCountryOfCitizenship").unwrap_or("")));

        if let ProfileRole::Student = self.role {
            if let Some(info) = self.decode_student_number() {
                let level = match info.study_level {
                    1 => "bachelor",
                    2 => "master",
                    3 => "doctoral",
                    _ => "other",
                };
                ctx.insert("enrolment_year".into(), serde_json::json!(info.enrolment_year));
                ctx.insert("study_level".into(), serde_json::json!(level));
            }
        }

        serde_json::Value::Object(ctx)
    }

    /// Decodes the student username (e.g. "89161125") into structured info.
    /// Format: XX YYY L SSS where XX=faculty, YYY=enrolment year, L=level, SSS=seq.
    fn decode_student_number(&self) -> Option<StudentInfo> {
        let raw = self.raw_attributes.get("uid")
            .or_else(|| self.raw_attributes.get("eduPersonNickname"))?
            .first()?;

        let digits: String = raw.chars().filter(|c| c.is_ascii_digit()).collect();
        if digits.len() < 8 { return None; }

        Some(StudentInfo {
            faculty_code: digits[0..2].to_string(),
            enrolment_year: digits[2..6].parse().ok()?,
            study_level: digits[6..7].parse().ok()?,
            seq_number: digits[7..].parse().ok()?,
        })
    }
}

fn extract_uid(ldap_resp: &str) -> Result<String, String> {
    let uid_chunk = ldap_resp.split(",").nth(0)
        .ok_or_else(|| String::from("Could not extract uid chunk from ldap string"))?;
    let username = uid_chunk.split("=").nth(1)
        .ok_or_else(|| String::from("Could not extract uid value from ldap string"))?;
    Ok(username.into())
}
