//! Robust extractor for FAMNIT “programme” pages.
//!
//! Feed the raw HTML of any programme page (`…/education/<cycle>/<slug>/…`)
//! into `ProgrammeInfo::from(html)` and it will give you a fully–populated
//! struct plus a nice Markdown renderer via `Display`.

use html_escape::decode_html_entities;
use scraper::{CaseSensitivity, ElementRef, Html, Node, Selector};
use std::{collections::{HashMap, HashSet}, fmt::{self, Write}};

/// One physical course row inside a timetable.
///
/// *The first two columns (course name & ECTS) are always present; the rest
///  depend on the concrete table (L / S / T / LW / FW …).  You get every cell
///  as-is so you can decide later how to display them.*


// Add this enum to programme.rs
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum ProgrammeSection {
    GeneralInfo,
    Coordinators,
    About,
    Goals,
    CourseStructure,
    FieldWork,
    CourseTables,
    AdmissionRequirements,
    TransferCriteria,
    AdvancementRequirements,
    CompletionRequirements,
    Competencies,
    EmploymentOpportunities,
}

// Helper to convert from string (which the LLM will provide) to the enum
impl ProgrammeSection {
    pub fn from_str(s: &str) -> Option<Self> {
        match s.to_lowercase().as_str() {
            "general_info" => Some(Self::GeneralInfo),
            "coordinators" => Some(Self::Coordinators),
            "about" => Some(Self::About),
            "goals" => Some(Self::Goals),
            "course_structure" => Some(Self::CourseStructure),
            "field_work" => Some(Self::FieldWork),
            "course_tables" => Some(Self::CourseTables),
            "admission_requirements" => Some(Self::AdmissionRequirements),
            "transfer_criteria" => Some(Self::TransferCriteria),
            "advancement_requirements" => Some(Self::AdvancementRequirements),
            "completion_requirements" => Some(Self::CompletionRequirements),
            "competencies" => Some(Self::Competencies),
            "employment_opportunities" => Some(Self::EmploymentOpportunities),
            _ => None,
        }
    }
}


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

        let mut course_tables = vec![];
        
        let table_sel = Selector::parse("div.content table").unwrap();
        let row_sel = Selector::parse("tr").unwrap();
        let th_sel = Selector::parse("th").unwrap();
        let td_sel = Selector::parse("td").unwrap();

        for table in doc.select(&table_sel) {
            let mut heading = String::new();
            let mut caption = String::new();

            // 1. Find the table's heading (title).
            // The correct way is to iterate through preceding siblings and take the first element.
            if let Some(prev_el) = table.prev_siblings().filter_map(ElementRef::wrap).next() {
                heading = text(&prev_el);
            }
            // Fallback for wrapped tables
            else if let Some(parent) = table.parent() {
                if let Some(parent_prev_el) = parent.prev_siblings().filter_map(ElementRef::wrap).next() {
                    heading = text(&parent_prev_el);
                }
            }

            // 2. Find the table's caption (legend).
            // Do the same with following siblings.
            if let Some(next_el) = table.next_siblings().filter_map(ElementRef::wrap).next() {
                let potential_caption = text(&next_el);
                // Use heuristics to ensure it's a legend.
                if potential_caption.to_lowercase().contains("l = lecture") || potential_caption.to_lowercase().contains("legend:") {
                    caption = potential_caption;
                }
            }
            // Fallback for wrapped tables
            else if let Some(parent) = table.parent() {
                if let Some(parent_next_el) = parent.next_siblings().filter_map(ElementRef::wrap).next() {
                    let potential_caption = text(&parent_next_el);
                    if potential_caption.to_lowercase().contains("l = lecture") || potential_caption.to_lowercase().contains("legend:") {
                        caption = potential_caption;
                    }
                }
            }

            // Clean up extraneous whitespace from the extracted text.
            heading = heading.split_whitespace().collect::<Vec<_>>().join(" ");
            caption = caption.split_whitespace().collect::<Vec<_>>().join(" ");

            // 3. Parse the table rows.
            let mut rows = Vec::new();

            // First, determine if this is a table that can be mapped to `CourseRow`.
            let header_texts: Vec<String> = table.select(&th_sel).map(|th| text(&th).to_lowercase()).collect();
            let is_parsable_course_table = header_texts.iter().any(|h| h.contains("ects")) && header_texts.iter().any(|h| h.contains("course"));

            if !is_parsable_course_table {
                continue;
            }

            for tr in table.select(&row_sel) {
                let cells: Vec<String> = tr
                    .select(&td_sel)
                    .map(|td| text(&td))
                    .collect::<Vec<_>>();

                if cells.len() < 2 {
                    continue;
                }

                let mut data_offset = 0;
                if cells[0].ends_with('.') && cells[0].trim_end_matches('.').parse::<u32>().is_ok() {
                    data_offset = 1;
                }

                let get_cell = |n: usize| -> String {
                    cells.get(data_offset + n).cloned().unwrap_or_else(String::new)
                };

                let num_data_cols = cells.len() - data_offset;
                
                if num_data_cols >= 6 {
                    rows.push(CourseRow {
                        course: get_cell(0),
                        ects:   get_cell(1),
                        l:      get_cell(2),
                        s:      get_cell(3),
                        t:      get_cell(4),
                        lw:     get_cell(5),
                        extra:  if num_data_cols > 7 { Some(get_cell(6)) } else { None },
                        total:  if num_data_cols > 7 { get_cell(7) } else { get_cell(6) },
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

impl ProgrammeInfo {
    /// Render the profile as a Markdown string, optionally filtered by sections.
    /// If `sections_to_render` is None or empty, all sections are rendered.
    pub fn to_markdown(&self, sections_to_render: Option<&HashSet<ProgrammeSection>>) -> String {
        let mut f = String::new();
        let should_render = |section: &ProgrammeSection| -> bool {
            sections_to_render.map_or(true, |s| s.is_empty() || s.contains(section))
        };

        // Helper closures for writing DRY code
        let write_paras = |f: &mut String, title: &str, paras: &[String]| {
            if !paras.is_empty() {
                writeln!(f, "## {}\n", title).unwrap();
                for p in paras {
                    writeln!(f, "{}\n", p).unwrap();
                }
            }
        };
        let write_list = |f: &mut String, title: &str, items: &[String]| {
            if !items.is_empty() {
                writeln!(f, "## {}\n", title).unwrap();
                for (i, item) in items.iter().enumerate() {
                    writeln!(f, "{}. {}", i + 1, item).unwrap();
                }
                writeln!(f).unwrap();
            }
        };

        // --- Render sections based on the filter ---

        writeln!(&mut f, "# {}\n", self.name).unwrap();

        if should_render(&ProgrammeSection::GeneralInfo) {
            writeln!(&mut f, "**Type:** {}", self.programme_type).unwrap();
            writeln!(&mut f, "**Degree awarded:** {}", self.degree_awarded).unwrap();
            writeln!(&mut f, "**Duration:** {}", self.duration).unwrap();
            writeln!(&mut f, "**ECTS credits:** {}", self.ects_credits).unwrap();
            writeln!(&mut f, "**Structure:** {}", self.structure).unwrap();
            writeln!(&mut f, "**Mode:** {}", self.mode_of_study).unwrap();
            writeln!(&mut f, "**Language:** {}\n", self.language_of_study).unwrap();
        }

        if should_render(&ProgrammeSection::Coordinators) {
            writeln!(&mut f, "## Programme coordinator(s)").unwrap();
            for (name, href) in &self.coordinators {
                writeln!(&mut f, "- [{}]({})", name, href).unwrap();
            }
            if let Some(link) = &self.student_services {
                writeln!(&mut f, "\nContact for admin procedures: [{}]({})\n", "Student Services", link).unwrap();
            }
            if let Some(link) = &self.course_description {
                writeln!(&mut f, "Short course descriptions: [{}]({})\n", "PDF", link).unwrap();
            }
        }

        if should_render(&ProgrammeSection::About) {
            write_paras(&mut f, "About the programme", &self.about);
        }
        if should_render(&ProgrammeSection::Goals) {
            write_list(&mut f, "Educational & professional goals", &self.goals);
        }
        if should_render(&ProgrammeSection::CourseStructure) {
            write_paras(&mut f, "Course structure (notes)", &self.course_structure_notes);
        }
        if should_render(&ProgrammeSection::FieldWork) {
            write_paras(&mut f, "Field work", &self.field_work);
        }

        if should_render(&ProgrammeSection::CourseTables) {
            writeln!(&mut f, "## Course tables").unwrap();
            for table in &self.course_tables {
                writeln!(&mut f, "### {}\n", table.title).unwrap();
                let has_extra = table.rows.first().and_then(|r| r.extra.as_ref()).is_some();
                writeln!(&mut f, "| Course | ECTS | L | S | T | LW{}| Total |", if has_extra { " | Extra " } else { "" }).unwrap();
                writeln!(&mut f, "|---|---|---|---|---|---{}|---|", if has_extra { "|---" } else { "" }).unwrap();
                for row in &table.rows {
                    write!(&mut f, "| {} | {} | {} | {} | {} | {}", row.course, row.ects, row.l, row.s, row.t, row.lw).unwrap();
                    if let Some(extra) = &row.extra {
                        write!(&mut f, " | {}", extra).unwrap();
                    }
                    writeln!(&mut f, " | {} |", row.total).unwrap();
                }
                writeln!(&mut f, "\n{}\n", table.caption).unwrap();
            }
        }

        if should_render(&ProgrammeSection::AdmissionRequirements) {
            write_list(&mut f, "Admission requirements", &self.admission_requirements);
        }
        if should_render(&ProgrammeSection::TransferCriteria) {
            write_list(&mut f, "Continuation of studies (transfer criteria)", &self.transfer_criteria);
        }
        if should_render(&ProgrammeSection::AdvancementRequirements) {
            write_paras(&mut f, "Advancement requirements", &self.advancement_requirements);
        }
        if should_render(&ProgrammeSection::CompletionRequirements) {
            if let Some(ref c) = self.completion_requirements {
                writeln!(&mut f, "### Requirements for completion of studies\n{}\n", c).unwrap();
            }
        }

        if should_render(&ProgrammeSection::Competencies) {
            if !self.competencies_general.is_empty() || !self.competencies_subject.is_empty() {
                writeln!(&mut f, "## Graduate competencies").unwrap();
                if !self.competencies_general.is_empty() {
                    writeln!(&mut f, "\n**General:**").unwrap();
                    for (i, g) in self.competencies_general.iter().enumerate() {
                        writeln!(&mut f, "{}. {}", i + 1, g).unwrap();
                    }
                }
                if !self.competencies_subject.is_empty() {
                    writeln!(&mut f, "\n**Subject-specific:**").unwrap();
                    for (i, g) in self.competencies_subject.iter().enumerate() {
                        writeln!(&mut f, "{}. {}", i + 1, g).unwrap();
                    }
                }
                writeln!(&mut f).unwrap();
            }
        }
        
        if should_render(&ProgrammeSection::EmploymentOpportunities) {
            write_paras(&mut f, "Graduate employment opportunities", &self.employment_opportunities);
        }

        f
    }
}

// The old Display trait now just calls the new method with no filter
impl fmt::Display for ProgrammeInfo {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.to_markdown(None))
    }
}