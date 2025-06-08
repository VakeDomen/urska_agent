//! Robust extractor for FAMNIT “programme” pages.
//!
//! Feed the raw HTML of any programme page (`…/education/<cycle>/<slug>/…`)
//! into `ProgrammeInfo::from(html)` and it will give you a fully–populated
//! struct plus a nice Markdown renderer via `Display`.

use html_escape::decode_html_entities;
use scraper::{CaseSensitivity, ElementRef, Html, Node, Selector};
use std::{collections::HashMap, fmt};

/// One physical course row inside a timetable.
///
/// *The first two columns (course name & ECTS) are always present; the rest
///  depend on the concrete table (L / S / T / LW / FW …).  You get every cell
///  as-is so you can decide later how to display them.*

#[derive(Debug)]
pub struct CourseRow {
    pub course: String,
    pub ects: String,
    pub l: String,
    pub s: String,
    pub t: String,
    pub lw: String,
    pub extra: Option<String>, // for columns like FW, SE, etc
    pub total: String,
}

#[derive(Debug)]
pub struct CourseTable {
    pub title: String,
    pub caption: String,
    pub rows: Vec<CourseRow>,
}
/// Complete programme record.
#[derive(Debug)]
pub struct ProgrammeInfo {
    // ── “General information” ───────────────────────────────────────────
    pub name:              String,
    pub programme_type:    String,
    pub degree_awarded:    String,
    pub duration:          String,
    pub ects_credits:      String,
    pub structure:         String,
    pub mode_of_study:     String,
    pub language_of_study: String,

    // ── misc links ──────────────────────────────────────────────────────
    pub coordinators:        Vec<(String, String)>,
    pub student_services:    Option<String>,
    pub course_description:  Option<String>,

    // ── narrative sections ──────────────────────────────────────────────
    pub about:                     Vec<String>,
    pub goals:                     Vec<String>,
    pub course_structure_notes:    Vec<String>,
    pub field_work:                Vec<String>,
    pub admission_requirements:    Vec<String>,
    pub transfer_criteria:         Vec<String>,
    pub advancement_requirements:  Vec<String>,
    pub completion_requirements:   Option<String>,
    pub competencies_general:      Vec<String>,
    pub competencies_subject:      Vec<String>,
    pub employment_opportunities:  Vec<String>,

    // ── tables ──────────────────────────────────────────────────────────
    pub course_tables: Vec<CourseTable>,
}

/* --------------------------------------------------------------------- */
/*  Helper utilities                                                     */
/* --------------------------------------------------------------------- */

/// Case–insensitive *substring* match:  does `haystack` contain *any* `needle`?
fn heading_is(haystack: &str, needle: &str) -> bool {
    haystack.to_ascii_lowercase()
            .contains(&needle.to_ascii_lowercase())
}

/// Collect plain‐text out of an ElementRef (recursively).
fn text(er: &ElementRef) -> String {
    er.text().collect::<Vec<_>>().join(" ").trim().to_string()
}

/// Collect all `<li>` immediate children as trimmed lines.
fn list_items(list: &ElementRef) -> Vec<String> {
    let li_sel = Selector::parse("li").unwrap();
    list.select(&li_sel)
        .map(|li| text(&li))
        .filter(|s| !s.is_empty())
        .collect()
}

/// Split a `<p>` that contains “Label: value<br>`” triples.
fn peel_value(lines: &[String], label: &str) -> String {
    lines
        .iter()
        .find_map(|l| {
            let lower = l.to_ascii_lowercase();
            if lower.starts_with(&label.to_ascii_lowercase()) {
                l.splitn(2, ':').nth(1).map(|s| s.trim().to_string())
            } else {
                None
            }
        })
        .unwrap_or_else(|| "—".into())
}

/* --------------------------------------------------------------------- */
/*  Main parser                                                          */
/* --------------------------------------------------------------------- */

impl From<String> for ProgrammeInfo {
    fn from(html: String) -> Self {
        let doc = Html::parse_document(&html);
        let h1_sel = Selector::parse("h1").unwrap();
        let content_sel = Selector::parse("div.content").unwrap();
        let a_sel = Selector::parse("a").unwrap();

        let name = text(
            &doc.select(&h1_sel)
                .next()
                .expect("programme page has <h1>"),
        );

        /* ---------- 1.  cut the document into sections by <h2> ---------- */

        let mut sections: HashMap<String, Vec<ElementRef>> = HashMap::new();
        let mut current: Option<String> = None;

        if let Some(content) = doc.select(&content_sel).next() {
            for node in content.children() {
                if let Some(er) = ElementRef::wrap(node) {
                    match er.value().name() {
                        "h2" => {
                            let title = text(&er);
                            current = Some(title.clone());
                            sections.entry(title).or_default();
                        }
                        _ => {
                            if let Some(ref key) = current {
                                sections.entry(key.clone()).or_default().push(er);
                            }
                        }
                    }
                }
            }
        }

        /* ---------- 2.  GENERAL INFORMATION (first P) ------------------- */
        // --- SELECTORS ---
        let h1_sel = Selector::parse("h1").unwrap();
        let p_sel  = Selector::parse("div.content > p").unwrap();
        let h2_sel = Selector::parse("div.content > h2").unwrap();
        let h3_sel = Selector::parse("div.content > h3").unwrap();
        let ul_sel = Selector::parse("div.content > ul, div.content > ol").unwrap();
        let a_sel  = Selector::parse("a").unwrap();
        let med_tbl_sel = Selector::parse("div.content").unwrap();
        let medium_or_table = Selector::parse("div.medium, table").unwrap();

        // --- 1. GENERAL INFO (first <p>) ---
        let general_html = doc
            .select(&p_sel)
            .next()
            .map(|p| p.inner_html())
            .unwrap_or_default();

        let mut lines = general_html
            .split("<br")
            .map(|chunk| decode_html_entities(chunk).trim().to_string())
            .filter(|s| !s.is_empty())
            .collect::<Vec<_>>();

        let mut peel = |label: &str| {
            lines
                .iter()
                .find(|l| l.to_lowercase().contains(&label.to_lowercase()))
                .and_then(|l| l.splitn(2, ':').nth(1))
                .map(|v| v.trim().to_string())
                .unwrap_or_else(|| "—".into())
        };

        let programme_type    = peel("type of programme");
        let degree_awarded    = peel("degree awarded");
        let duration          = peel("duration");
        let ects_credits      = peel("ects-credits");
        let structure         = peel("programme structure");
        let mode_of_study     = peel("mode of study");
        let language_of_study = peel("language of study");

        /* ---------- 3.  Links & coordinators --------------------------- */

        let mut coordinators = Vec::<(String, String)>::new();
        let mut student_services = None;
        let mut course_description = None;

        // pull every <a> in the whole document once:
        for a in doc.select(&a_sel) {
            let href = a.value().attr("href").unwrap_or("").to_string();
            let txt = text(&a);

            // Programme coordinators (section check)
            if sections
                .iter()
                .any(|(k, nodes)| heading_is(k, "programme coordinator")
                    && nodes.iter().any(|n| n.id() == a.parent().unwrap().id()))
            {
                coordinators.push((txt.clone(), href.clone()));
            }

            // Student Services link
            if txt.to_ascii_lowercase().contains("student services") {
                student_services.get_or_insert(href.clone());
            }

            // “Short descriptions HERE”
            if txt.to_ascii_lowercase().contains("short description")
                || txt.to_ascii_lowercase() == "here"
            {
                course_description.get_or_insert(href.clone());
            }
        }

        /* ---------- 4.  Helper closures over sections ------------------ */

        let grab_paragraphs = |title_keyword: &str| -> Vec<String> {
            sections
                .iter()
                .find(|(k, _)| heading_is(k, title_keyword))
                .map(|(_, nodes)| {
                    nodes
                        .iter()
                        .filter(|er| er.value().name() == "p")
                        .map(|p| text(p))
                        .filter(|s| !s.is_empty())
                        .collect()
                })
                .unwrap_or_default()
        };

        let grab_list = |title_keyword: &str| -> Vec<String> {
            sections
                .iter()
                .find(|(k, _)| heading_is(k, title_keyword))
                .map(|(_, nodes)| {
                    nodes
                        .iter()
                        .filter(|er| {
                            let n = er.value().name();
                            n == "ul" || n == "ol"
                        })
                        .flat_map(|ul| list_items(ul))
                        .collect()
                })
                .unwrap_or_default()
        };

        /* ---------- 5.  Tables (div.medium + table) -------------------- */

        let doc = Html::parse_document(&html);
        let table_sel = Selector::parse("div.content > div > table").unwrap();
        let mut course_tables = vec![];
        let table_sel = Selector::parse("div.content table").unwrap();
        let row_sel   = Selector::parse("tr").unwrap();
        let cell_sel  = Selector::parse("td, th").unwrap();
        let div_sel   = Selector::parse("div").unwrap();

        for table in doc.select(&table_sel) {
            // 1) Find the <div> that wraps this table
            let wrapper_div = table
                .parent()
                .and_then(ElementRef::wrap)
                .expect("table is not inside a <div>");

            let mut seen_table = false;
            let mut heading = String::new();
            let mut caption = String::new();

            // 2) Iterate *only* over that div’s direct children
            if wrapper_div.children().count() > 4 {
                // 1) First look for a sibling <div class="medium"> just before the table
                println!("{:#?}", table.prev_sibling().map(|e| e.value().is_element()));
                if let Some(prev_sib) = table
                    .prev_sibling()
                    .and_then(ElementRef::wrap)
                    .filter(|e| {println!("FILTER"); e.value().has_class("medium", CaseSensitivity::AsciiCaseInsensitive)})
                {
                    println!("PREV TRUE");

                    heading = prev_sib.text().collect::<Vec<_>>().join(" ").trim().to_string();
                }
                
                // 2) Otherwise, fall back to the generic “everything before the table” approach
                else {
                    let mut seen_table = false;
                    for child in wrapper_div.children() {
                        if let Node::Element(el) = child.value() {
                            if el.name.local.as_ref() == "table" {
                                seen_table = true;
                                continue;
                            }
                        }
                        let txt = child
                            .descendants()
                            .filter_map(|n| if let Node::Text(t) = n.value() {
                                let s = t.trim();
                                if !s.is_empty() { Some(s) } else { None }
                            } else { None })
                            .collect::<Vec<_>>()
                            .join(" ");
                        if txt.is_empty() { continue }
                        if !seen_table {
                            heading.push_str(&txt);
                            heading.push(' ');
                        } else {
                            caption.push_str(&txt);
                            caption.push(' ');
                        }
                    }
                    heading = heading.trim().to_string();
                    caption = caption.trim().to_string();
                }
            } else {

                for child in wrapper_div.children() {
                    match child.value() {
                        // Once we hit the <table> tag, switch to caption mode
                        Node::Element(el) if el.name.local.as_ref() == "table" => {
                            seen_table = true;
                            continue;
                        }
                        _ => {
                            // Collect *all* text under this child by walking its descendants
                            let txt = child
                                .descendants()
                                .filter_map(|n| {
                                    if let Node::Text(t) = n.value() {
                                        let s = t.trim();
                                        if !s.is_empty() { Some(s) } else { None }
                                    } else {
                                        None
                                    }
                                })
                                .collect::<Vec<_>>()
                                .join(" ");
    
                            if txt.is_empty() {
                                continue;
                            }
                            if !seen_table {
                                heading.push_str(&txt);
                                heading.push(' ');
                            } else {
                                caption.push_str(&txt);
                                caption.push(' ');
                            }
                        }
                    }
                }
            }


            let heading = heading.trim().to_string();
            let caption = caption.trim().to_string();

            // 3. Parse rows as before
            let mut rows = Vec::new();
            for tr in table.select(&row_sel) {
                let cells = tr
                    .select(&cell_sel)
                    .map(|td| td.text().collect::<String>().trim().to_string())
                    .collect::<Vec<_>>();
                // adapt to whatever column count you need
                if cells.len() >= 7 {
                    rows.push(CourseRow {
                        course: cells[0].clone(),
                        ects:   cells[1].clone(),
                        l:      cells[2].clone(),
                        s:      cells[3].clone(),
                        t:      cells[4].clone(),
                        lw:     cells[5].clone(),
                        total:  cells[6].clone(),
                        extra:  if cells.len() == 8 { Some(cells[6].clone()) } else { None },
                    });
                }
            }

            if !rows.is_empty() {
                course_tables.push(CourseTable { title: heading, rows, caption });
            }
        }
        /* ---------- 6.  Remaining narrative sections ------------------- */

        let about                  = grab_paragraphs("about the programme");
        let goals                  = grab_list("educational and professional goals");
        let course_structure_notes = grab_paragraphs("course structure");
        let field_work             = grab_paragraphs("field work");
        let admission_requirements = grab_list("admission requirements");
        let transfer_criteria      = grab_list("continuation of studies");
        let advancement_requirements = grab_paragraphs("advancement requirements");

        // completion requirements (h3 + p)
        let h3_sel = Selector::parse("h3").unwrap();
        let completion_requirements = doc
            .select(&h3_sel)
            .find(|h| heading_is(&text(&h), "requirements for the completion"))
            .and_then(|h3| {
                // first following <p>
                h3.next_sibling()
                    .and_then(ElementRef::wrap)
                    .filter(|er| er.value().name() == "p")
            })
            .map(|p| text(&p));

        // competencies: two <ul> one after another
        let competencies_general   = grab_list("graduate competencies")
            .into_iter()
            .take_while(|_| true) // keeps them separated in code below
            .collect::<Vec<_>>();

        // after the first <h3>   we collect next <ul>
        let competencies_subject = sections
            .iter()
            .find(|(k, _)| heading_is(k, "graduate competencies"))
            .and_then(|(_, nodes)| {
                nodes
                    .iter()
                    .filter(|er| er.value().name() == "ul" || er.value().name() == "ol")
                    .nth(1)
            })
            .map(|ul| list_items(ul))
            .unwrap_or_default();

        let employment_opportunities = grab_paragraphs("employment opportunities");

        /* ---------- 7.  Build struct ----------------------------------- */

        ProgrammeInfo {
            name,
            programme_type,
            degree_awarded,
            duration,
            ects_credits,
            structure,
            mode_of_study,
            language_of_study,
            coordinators,
            student_services,
            course_description,
            about,
            goals,
            course_structure_notes,
            field_work,
            admission_requirements,
            transfer_criteria,
            advancement_requirements,
            completion_requirements,
            competencies_general,
            competencies_subject,
            employment_opportunities,
            course_tables,
        }
    }
}

/* --------------------------------------------------------------------- */
/*  Markdown renderer                                                    */
/* --------------------------------------------------------------------- */

impl fmt::Display for ProgrammeInfo {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(f, "# {}\n", self.name)?;

        writeln!(f, "**Type:** {}", self.programme_type)?;
        writeln!(f, "**Degree awarded:** {}", self.degree_awarded)?;
        writeln!(f, "**Duration:** {}", self.duration)?;
        writeln!(f, "**ECTS credits:** {}", self.ects_credits)?;
        writeln!(f, "**Structure:** {}", self.structure)?;
        writeln!(f, "**Mode:** {}", self.mode_of_study)?;
        writeln!(f, "**Language:** {}\n", self.language_of_study)?;

        writeln!(f, "## Programme coordinator(s)")?;
        for (name, href) in &self.coordinators {
            writeln!(f, "- [{}]({})", name, href)?;
        }
        if let Some(link) = &self.student_services {
            writeln!(
                f,
                "\nContact for admin procedures: [{}]({})\n",
                "Student Services", link
            )?;
        }
        if let Some(link) = &self.course_description {
            writeln!(
                f,
                "Short course descriptions: [{}]({})\n",
                "PDF",
                link
            )?;
        }

        let write_paras = |f: &mut fmt::Formatter<'_>, title: &str, paras: &[String]| -> fmt::Result {
            if !paras.is_empty() {
                writeln!(f, "## {}\n", title)?;
                for p in paras {
                    writeln!(f, "{}\n", p)?;
                }
            }
            Ok(())
        };

        write_paras(f, "About the programme", &self.about)?;
        if !self.goals.is_empty() {
            writeln!(f, "## Educational & professional goals")?;
            for (i, g) in self.goals.iter().enumerate() {
                writeln!(f, "{}. {}", i + 1, g)?;
            }
            writeln!(f)?;
        }

        write_paras(f, "Course structure (notes)", &self.course_structure_notes)?;
        write_paras(f, "Field work", &self.field_work)?;

        writeln!(f, "## Course tables")?;
        for table in &self.course_tables {
            writeln!(f, "### {}\n", table.title)?;
            writeln!(f, "| Course | ECTS | L | S | T | LW{}| Total |",
                if table.rows.first().and_then(|r| r.extra.as_ref()).is_some() { " | Extra " } else { "" }
            )?;
            writeln!(f, "|---|---|---|---|---|---{}|---|",
                if table.rows.first().and_then(|r| r.extra.as_ref()).is_some() { "|---" } else { "" }
            )?;
            for row in &table.rows {
                write!(
                    f,
                    "| {} | {} | {} | {} | {} | {}",
                    row.course, row.ects, row.l, row.s, row.t, row.lw
                )?;
                if let Some(extra) = &row.extra {
                    write!(f, " | {}", extra)?;
                }
                writeln!(f, " | {} |", row.total)?;
            }
            writeln!(f)?;
            writeln!(f, "{}", table.caption)?;
            writeln!(f)?;
            writeln!(f)?;
        }

        if !self.admission_requirements.is_empty() {
            writeln!(f, "## Admission requirements")?;
            for (i, r) in self.admission_requirements.iter().enumerate() {
                writeln!(f, "{}. {}", i + 1, r)?;
            }
            writeln!(f)?;
        }

        if !self.transfer_criteria.is_empty() {
            writeln!(f, "## Continuation of studies (transfer criteria)")?;
            for (i, r) in self.transfer_criteria.iter().enumerate() {
                writeln!(f, "{}. {}", i + 1, r)?;
            }
            writeln!(f)?;
        }

        write_paras(f, "Advancement requirements", &self.advancement_requirements)?;

        if let Some(ref c) = self.completion_requirements {
            writeln!(f, "### Requirements for completion of studies\n{}\n", c)?;
        }

        if !self.competencies_general.is_empty() || !self.competencies_subject.is_empty() {
            writeln!(f, "## Graduate competencies")?;
            if !self.competencies_general.is_empty() {
                writeln!(f, "\n**General:**")?;
                for (i, g) in self.competencies_general.iter().enumerate() {
                    writeln!(f, "{}. {}", i + 1, g)?;
                }
            }
            if !self.competencies_subject.is_empty() {
                writeln!(f, "\n**Subject-specific:**")?;
                for (i, g) in self.competencies_subject.iter().enumerate() {
                    writeln!(f, "{}. {}", i + 1, g)?;
                }
            }
            writeln!(f)?;
        }

        write_paras(f, "Graduate employment opportunities", &self.employment_opportunities)?;

        Ok(())
    }
}

