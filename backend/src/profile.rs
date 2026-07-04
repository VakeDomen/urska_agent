use std::collections::HashMap;
use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
pub enum ProfileRole {
    Student,
    Employee,
    Admin,
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
}

fn extract_uid(ldap_resp: &str) -> Result<String, String> {
    let uid_chunk = ldap_resp.split(",").nth(0)
        .ok_or_else(|| String::from("Could not extract uid chunk from ldap string"))?;
    let username = uid_chunk.split("=").nth(1)
        .ok_or_else(|| String::from("Could not extract uid value from ldap string"))?;
    Ok(username.into())
}
