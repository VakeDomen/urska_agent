use serde::Serialize;


#[derive(Debug, Serialize)]
pub enum ProfileRole {
    Student,
    Employee,
    Admin,
}

#[derive(Debug, Serialize)]
pub struct Profile {
    role: ProfileRole,
    username: String,
}
