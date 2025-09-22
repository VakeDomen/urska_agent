use serde::Serialize;


#[derive(Debug, Clone, Serialize)]
pub enum ProfileRole {
    Student,
    Employee,
    Admin,
}

#[derive(Debug, Clone, Serialize)]
pub struct Profile {
    role: ProfileRole,
    username: String,
}

impl Profile {
    pub fn try_from_student_string(ldap_resp: String) -> Result<Self, String> {
        let Some(uid_chunk) = ldap_resp
            .split(",")
            .nth(0) else {
                return Err("Could not extract uid chunk from student ldap string".into())
            };


        let Some(username) = uid_chunk
            .split("=")
            .nth(1) else {
                return Err("Could not extract uid value from student ldap string".into())
            };
        
        Ok(Self { 
            role: ProfileRole::Student, 
            username: username.into() 
        })
    }

    pub fn try_from_employee_string(ldap_resp: String) -> Result<Self, String> {
        let Some(uid_chunk) = ldap_resp
            .split(",")
            .nth(0) else {
                return Err("Could not extract uid chunk from employee ldap string".into())
            };


        let Some(username) = uid_chunk
            .split("=")
            .nth(1) else {
                return Err("Could not extract uid value from employee ldap string".into())
            };
        
        Ok(Self { 
            role: ProfileRole::Employee, 
            username: username.into() 
        })
    }
}
