use ldap3::{Ldap, LdapConnAsync, Scope, SearchEntry};
use ldap3::result::Result;
use std::env;



pub async fn stdent_ldap_login(username: String, password: String) -> Result<Option<String>> {
    let (conn, ldap_conn) = LdapConnAsync::new(&env::var("LDAP_SERVER_STUDENT")
        .expect("$LDAP_SERVER is not set"))
        .await?;

    ldap3::drive!(conn);
    println!("LDAP connection established...");
    check_login(ldap_conn, username, password).await
}


pub async fn employee_ldap_login(username: String, password: String) -> Result<Option<String>> {
    println!("LDAP connection attempt...");
    let (conn, ldap_conn) = LdapConnAsync::new(&env::var("LDAP_SERVER_EMPLOYEE")
        .expect("$LDAP_SERVER is not set"))
        .await?;
    println!("LDAP connection drive...");
    ldap3::drive!(conn);
    println!("LDAP connection established...");
    check_login(ldap_conn, username, password).await
}

async fn check_login(
    mut ldap_conn: Ldap,
    username: String,
    password: String,

) -> Result<Option<String>> {
    // Search for the user in the directory
    let (rs, _res) = ldap_conn.search(
        "dc=upr,dc=si",
        Scope::Subtree,
        format!("(uid={})", username).as_str(),
        vec!["dn", "sn", "cn"]
    ).await?.success()?;


    let mut user_entry = None;
    // there should only be one entry in the array or results
    for entry in rs {
        let entry = SearchEntry::construct(entry);
        match ldap_conn.simple_bind(&entry.dn, &password).await {
            /*
                LdapError has a variant called RC(u32, String). This variant 
                represents an error with a specific error code (rc) returned 
                by the LDAP server.
                The error code (rc) is a numerical value that indicates the 
                type of error that occurred. LDAP defines a set of standard 
                error codes that can be used to indicate different types of 
                errors. For example, error code 49 is used to indicate that 
                the provided credentials (username or password) are invalid
            */
            Ok(r) => if r.rc == 0 { user_entry = Some(entry.dn); },
            Err(e) => println!("Error binding to ldap: {:?}", e)
        }
    }
    Ok(user_entry)
}