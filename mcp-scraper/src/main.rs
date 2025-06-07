use reqwest::Url;
use rmcp::{model::{CallToolResult, Content, ServerCapabilities, ServerInfo}, schemars, serde, tool, transport::SseServer, ServerHandler};
use anyhow::Result;
use scraper::{ElementRef, Html, Node, Selector};
use serde::Deserialize;

const BIND_ADDRESS: &str = "127.0.0.1:8000";

#[tokio::main]
async fn main() -> Result<()> {
    let ct = SseServer::serve(BIND_ADDRESS.parse()?)
        .await?
        .with_service(Service::new);

    tokio::signal::ctrl_c().await?;
    ct.cancel();

    Ok(())
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct StructRequest {
    pub url: String,
}


#[derive(Debug, Clone)]
struct Service;

#[tool(tool_box)]
impl Service {
    pub fn new() -> Self {
        Self {}
    }

    #[tool(description = "Get the current page content")]
    pub async fn get_web_page_content(
        &self, 
        #[tool(aggr)] url_arg: StructRequest,
    ) -> Result<CallToolResult, rmcp::Error> {
        let content = match extract_and_absolutize_div_content(&url_arg.url).await {
            Ok(Some(html_output)) => {
                //html2md::rewrite_html(&html_output, false)
		html_output
            }
            Ok(None) => return Err(rmcp::Error::new(
                rmcp::model::ErrorCode::INVALID_PARAMS, 
                "No content found", 
                None
            )),
            Err(e) => return Err(rmcp::Error::new(
                rmcp::model::ErrorCode::INTERNAL_ERROR, 
                e.to_string(), 
                None
            )),
        };
        Ok(CallToolResult::success(vec![Content::text(
            content
        )]))
    }
}

#[tool(tool_box)]
impl ServerHandler for Service {
    fn get_info(&self) -> ServerInfo {
        ServerInfo {
            instructions: Some("A simple UP FAMNIT webpage scraper".into()),
            capabilities: ServerCapabilities::builder()
                .enable_tools()
                .build(),
            ..Default::default()
        }
    }
}


/// Fetches a webpage, looks for a specific div (e.g., `div.app`),
/// and returns its HTML content with all internal links (href, src) made absolute.
///
/// # Arguments
/// * `page_url_str`: The URL of the page to process.
///
/// # Returns
/// * `Ok(Some(String))` containing the processed HTML of the div if found.
/// * `Ok(None)` if the target div is not found.
/// * `Err(String)` if any error occurs during fetching, parsing, or processing.
pub async fn extract_and_absolutize_div_content(page_url_str: &str) -> Result<Option<String>, String> {
    // Parse the page URL. This will also serve as the base for resolving relative links.
    let base_url = match Url::parse(page_url_str) {
        Ok(url) => url,
        Err(e) => return Err(format!("Invalid page URL '{}': {}", page_url_str, e)),
    };

    // Fetch the page content using reqwest
    let response = match reqwest::get(page_url_str).await {
        Ok(resp) => resp,
        Err(e) => return Err(format!("Failed to fetch URL '{}': {}", page_url_str, e)),
    };

    // Check if the request was successful
    if !response.status().is_success() {
        return Err(format!(
            "Request to '{}' failed with status: {}",
            page_url_str,
            response.status()
        ));
    }

    // Read the response body as text
    let html_content = match response.text().await {
        Ok(text) => text,
        Err(e) => return Err(format!("Failed to read response text from '{}': {}", page_url_str, e)),
    };

    // Parse the HTML document using the scraper crate
    let document = Html::parse_document(&html_content);

    // Define the CSS selector for the target div.
    // The Python code: `soup.find('div', class_=lambda x: x and set(x.split()).issuperset({"app"}))`
    // This finds a div that has the class "app". It can have other classes as well.
    // The equivalent CSS selector is "div.app".
    let div_selector_str = "div.app";
    let div_selector = match Selector::parse(div_selector_str) {
        Ok(selector) => selector,
        // This error should ideally not happen for a hardcoded valid selector.
        Err(_) => return Err(format!("Internal error: Invalid CSS selector: {}", div_selector_str)),
    };

    // Find the first div element matching the selector
    if let Some(content_div_element_ref) = document.select(&div_selector).next() {
        // If the div is found, reconstruct its HTML with absolute links
        let processed_html = reconstruct_element_html_with_absolute_links(content_div_element_ref, &base_url);
        Ok(Some(processed_html))
    } else {
        // Target div was not found on the page
        Ok(None)
    }
}

fn reconstruct_element_html_with_absolute_links(element: ElementRef, base_url: &Url) -> String {
    let tag_name = element.value().name();

    // Skip <script> and <style> tags entirely
    if tag_name.eq_ignore_ascii_case("script") || tag_name.eq_ignore_ascii_case("style") {
        return String::new();
    }

    // Skip <link rel="stylesheet"> tags
    if tag_name.eq_ignore_ascii_case("link") {
        if let Some(rel_value) = element.value().attr("rel") {
            if rel_value.eq_ignore_ascii_case("stylesheet") {
                return String::new();
            }
        }
    }

    let mut html = String::new();

    // Start tag
    html.push('<');
    html.push_str(tag_name);

    for (name, value) in element.value().attrs() {
        if name.eq_ignore_ascii_case("style") {
            continue; // Remove inline style attributes
        }

        html.push(' ');
        html.push_str(name);
        html.push_str("=\"");

        if name == "href" || name == "src" {
            match base_url.join(value) {
                Ok(abs_url) => html.push_str(abs_url.as_str()),
                Err(_) => html.push_str(value),
            }
        } else {
            html.push_str(value);
        }

        html.push('"');
    }

    html.push('>');

    // Children nodes
    for child_node_ref in element.children() {
        match child_node_ref.value() {
            Node::Text(text_node) => {
                html.push_str(&text_node.text);
            }
            Node::Element(_) => {
                if let Some(child_element_ref) = ElementRef::wrap(child_node_ref) {
                    html.push_str(&reconstruct_element_html_with_absolute_links(child_element_ref, base_url));
                }
            }
            Node::Comment(_) => {} // Skip comments for now
            _ => {}
        }
    }

    // End tag
    html.push_str("</");
    html.push_str(tag_name);
    html.push('>');

    html
}

