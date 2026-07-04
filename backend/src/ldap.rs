use ldap3::{Ldap, LdapConnAsync, Scope, SearchEntry};
use ldap3::result::Result;
use std::collections::HashMap;
use std::env;

pub async fn stdent_ldap_login(username: String, password: String) -> Result<Option<HashMap<String, Vec<String>>>> {
    let (conn, ldap_conn) = LdapConnAsync::new(&env::var("LDAP_SERVER_STUDENT")
        .expect("$LDAP_SERVER is not set"))
        .await?;

    ldap3::drive!(conn);
    println!("LDAP connection established...");
    check_login(ldap_conn, username, password).await
}

pub async fn employee_ldap_login(username: String, password: String) -> Result<Option<HashMap<String, Vec<String>>>> {
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
) -> Result<Option<HashMap<String, Vec<String>>>> {
    let (rs, _res) = ldap_conn.search(
        "dc=upr,dc=si",
        Scope::Subtree,
        format!("(uid={})", username).as_str(),
        vec!["*"]
    ).await?.success()?;

    for raw in rs {
        let entry = SearchEntry::construct(raw);
        match ldap_conn.simple_bind(&entry.dn, &password).await {
            Ok(r) if r.rc == 0 => {
                let mut attrs = entry.attrs;
                attrs.insert("dn".into(), vec![entry.dn]);
                return Ok(Some(attrs));
            }
            Err(e) => println!("Error binding to ldap: {:?}", e),
            _ => {}
        }
    }
    Ok(None)
}