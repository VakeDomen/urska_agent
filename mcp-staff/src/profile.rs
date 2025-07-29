use scraper::{Html, Selector};
use std::fmt;


/// A single staff-profile as shown on the
/// https://www.famnit.upr.si … /staff/<name> pages.
#[derive(Debug)]
pub struct StaffProfile {
    pub full_name: String,
    pub title_sl: String,
    pub title_en: String,
    pub office: Option<String>,
    pub phone: Option<String>,
    pub email: Option<String>,
    pub website: Option<String>,
    pub department_sl: Option<String>,
    pub department_en: Option<String>,
    pub research_fields: Vec<String>,
    pub bibliography_si: Option<String>,
    pub bibliography_en: Option<String>,
    pub teaching_sl: Vec<String>,
    pub teaching_en: Vec<String>,
    pub coordinator_sl: Vec<String>,
    pub coordinator_en: Vec<String>,
    pub _photo_url: Option<String>,
}

impl From<String> for StaffProfile {
    fn from(html: String) -> Self {
        let doc = Html::parse_document(&html);

        // handy helpers -----------------------------------------------------
        let first_text = |sel: &str| -> Option<String> {
            let sel = Selector::parse(sel).unwrap();
            doc.select(&sel)
                .next()
                .map(|n| n.text().collect::<String>().trim().to_owned())
                .filter(|s| !s.is_empty())
        };

        let attr = |sel: &str, attr_name: &str| -> Option<String> {
            let sel = Selector::parse(sel).unwrap();
            doc.select(&sel)
                .next()
                .and_then(|n| n.value().attr(attr_name))
                .map(|s| s.to_owned())
        };

        // --------------------------------------------------------------------

        // top-level name & title
        let full_name = first_text("h1[itemprop=\"name\"]").unwrap_or_default();

        let h2_sel = Selector::parse("h2[itemprop=\"title\"]").unwrap();
        let (title_sl, title_en) = if let Some(node) = doc.select(&h2_sel).next() {
            let sl = node
                .text()
                .next()
                .map(|t| t.trim().to_owned())
                .unwrap_or_default();
            let en = node
                .select(&Selector::parse("span").unwrap())
                .next()
                .map(|s| s.text().collect::<String>().trim().to_owned())
                .unwrap_or_default();
            (sl, en)
        } else {
            (String::new(), String::new())
        };

        // simple scalar fields
        let office        = first_text("td.kabinet");
        let phone         = first_text("td.phone");
        let email         = first_text("td.email a");
        let website       = attr("td.website a", "href");
        let _photo_url     = attr("img.person-img-pedagoska", "src");

        // department (two <div class="field"> nodes, SLO → ENG)
        let binding = Selector::parse("td.departments .field").unwrap();
        let mut deps = doc
            .select(&binding)
            .map(|n| n.text().collect::<String>().trim().to_owned());
        let department_sl = deps.next();
        let department_en = deps.next();

        // research areas
        let research_fields = doc
            .select(&Selector::parse("td.research .field").unwrap())
            .map(|n| n.text().collect::<String>().trim().to_owned())
            .collect::<Vec<_>>();

        // bibliography SI / EN links
        let mut bibliography_si = None;
        let mut bibliography_en = None;
        for a in doc.select(&Selector::parse("td.bibliography a").unwrap()) {
            let label = a.text().collect::<String>().trim().to_uppercase();
            let href  = a.value().attr("href").unwrap_or("").to_owned();
            match label.as_str() {
                "SI" => bibliography_si = Some(href),
                "EN" => bibliography_en = Some(href),
                _    => (),
            }
        }

        // helper that splits "SLO / English" blocks --------------------------
        fn split_pair(block: &str) -> (String, String) {
            // e.g. "Računalništvo / Computer Science"
            let parts: Vec<_> = block.split('/').map(|s| s.trim().to_owned()).collect();
            let sl = parts.get(0).cloned().unwrap_or_default();
            let en = parts.get(1).cloned().unwrap_or_default();
            (sl, en)
        }

        // teaching courses
        let mut teaching_sl = Vec::new();
        let mut teaching_en = Vec::new();
        for field in doc.select(&Selector::parse("td.subjects .field").unwrap()) {
            let (sl, en) = split_pair(&field.text().collect::<String>());
            if !sl.is_empty() { teaching_sl.push(sl) }
            if !en.is_empty() { teaching_en.push(en) }
        }

        // coordinator roles (same pattern)
        let mut coordinator_sl = Vec::new();
        let mut coordinator_en = Vec::new();
        for field in doc.select(&Selector::parse("tr.last td.text .field").unwrap()) {
            let (sl, en) = split_pair(&field.text().collect::<String>());
            if !sl.is_empty() { coordinator_sl.push(sl) }
            if !en.is_empty() { coordinator_en.push(en) }
        }

        // assemble -----------------------------------------------------------
        StaffProfile {
            full_name,
            title_sl,
            title_en,
            office,
            phone,
            email,
            website,
            department_sl,
            department_en,
            research_fields,
            bibliography_si,
            bibliography_en,
            teaching_sl,
            teaching_en,
            coordinator_sl,
            coordinator_en,
            _photo_url,
        }
    }
}


impl StaffProfile {
    /// Render the profile as a Markdown string.
    pub fn to_markdown(&self) -> String {
        // ------- collect rows we actually have -----------------------------
        let mut rows: Vec<(String, String)> = Vec::new();

        rows.push((
            "Title".into(),
            format!("{} / {}", self.title_sl, self.title_en).trim().into(),
        ));

        if let Some(ref office) = self.office {
            rows.push(("Office".into(), office.clone()));
        }
        if let Some(ref phone) = self.phone {
            rows.push(("Phone".into(), phone.clone()));
        }
        if let Some(ref email) = self.email {
            rows.push((
                "Email".into(),
                format!("[{}](mailto:{})", email, email),
            ));
        }
        if let Some(ref site) = self.website {
            rows.push(("Website".into(), site.clone()));
        }
        if self.department_sl.is_some() || self.department_en.is_some() {
            rows.push((
                "Department".into(),
                format!(
                    "{}{}{}",
                    self.department_sl.clone().unwrap_or_default(),
                    if self.department_en.is_some() { " / " } else { "" },
                    self.department_en.clone().unwrap_or_default()
                ),
            ));
        }
        if !self.research_fields.is_empty() {
            rows.push((
                "Research".into(),
                self.research_fields.join(", "),
            ));
        }
        if self.bibliography_si.is_some() || self.bibliography_en.is_some() {
            let mut links = Vec::new();
            if let Some(ref si) = self.bibliography_si {
                links.push(format!("[SI]({})", si));
            }
            if let Some(ref en) = self.bibliography_en {
                links.push(format!("[EN]({})", en));
            }
            rows.push(("Bibliography".into(), links.join(" · ")));
        }
        if !self.teaching_sl.is_empty() || !self.teaching_en.is_empty() {
            rows.push((
                "Teaching".into(),
                format!(
                    "{}{}{}",
                    self.teaching_sl.join("; "),
                    if !self.teaching_en.is_empty() { " / " } else { "" },
                    self.teaching_en.join("; ")
                ),
            ));
        }
        if !self.coordinator_sl.is_empty() || !self.coordinator_en.is_empty() {
            rows.push((
                "Coordinator".into(),
                format!(
                    "{}{}{}",
                    self.coordinator_sl.join("; "),
                    if !self.coordinator_en.is_empty() { " / " } else { "" },
                    self.coordinator_en.join("; ")
                ),
            ));
        }

        // ------- build the markdown ---------------------------------------
        let mut md = String::new();
        md.push_str(&format!("### {}\n\n", self.full_name));
        md.push_str("| Field | Value |\n|-------|-------|\n");
        for (k, v) in rows {
            // escape vertical bars in the value so the table stays intact
            let safe = v.replace('|', "\\|");
            md.push_str(&format!("| {} | {} |\n", k, safe.trim()));
        }
        md
    }
}

impl fmt::Display for StaffProfile {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.to_markdown())
    }
}
