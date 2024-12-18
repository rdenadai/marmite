use crate::config::Marmite;
use crate::content::{
    get_authors, get_date, get_description, get_slug, get_stream, get_tags, get_title, Content,
};
use crate::site::Data;
use chrono::Datelike;
use comrak::{markdown_to_html, ComrakOptions};
use frontmatter_gen::{detect_format, extract_raw_frontmatter, parse, Frontmatter};
use regex::Regex;
use std::collections::HashMap;
use std::fs;
use std::path::Path;

/// Process the file, extract the content and add it to the site data
/// If the file is a post, add it to the posts vector
/// If the file is a page, add it to the pages vector
/// Also add the post to the tag and archive maps
pub fn process_file(
    path: &Path,
    site_data: &mut Data,
    fragments: &HashMap<String, String>,
) -> Result<(), String> {
    let content = get_content(path, Some(fragments), &site_data.site)?;

    if let Some(date) = content.date {
        site_data.posts.push(content.clone());
        // tags
        for tag in content.tags.clone() {
            site_data.tag.entry(tag).or_default().push(content.clone());
        }
        // authors
        for username in content.authors.clone() {
            site_data
                .author
                .entry(username)
                .or_default()
                .push(content.clone());
        }
        // archive by year
        let year = date.year().to_string();
        site_data
            .archive
            .entry(year)
            .or_default()
            .push(content.clone());
        // stream by name
        if let Some(stream) = &content.stream {
            site_data
                .stream
                .entry(stream.to_string())
                .or_default()
                .push(content.clone());
        };
    } else {
        site_data.pages.push(content);
    }
    Ok(())
}

pub fn append_references(content: &str, references_path: &Path) -> String {
    if references_path.exists() {
        let references = fs::read_to_string(references_path).unwrap_or_default();
        format!("{content}\n\n{references}")
    } else {
        content.to_string()
    }
}

/// From the file content, extract the frontmatter and the markdown content
/// then parse the markdown content to html and return a Content struct
/// if the file is a fragment, the markdown content will be modified to include the references
/// if is a regular content then content will be modified to include the `markdown_header`
/// and `markdown_footer` and references
pub fn get_content(
    path: &Path,
    fragments: Option<&HashMap<String, String>>,
    site: &Marmite,
) -> Result<Content, String> {
    let file_content = fs::read_to_string(path).map_err(|e| e.to_string())?;
    let (frontmatter, raw_markdown) = parse_front_matter(&file_content)?;
    let (title, markdown_without_title) = get_title(&frontmatter, raw_markdown);

    let is_fragment = path.file_name().unwrap().to_str().unwrap().starts_with('_');
    let html = if is_fragment {
        let references_path = path.with_file_name("_references.md");
        let mut raw_markdown = raw_markdown.to_string();
        if path != references_path {
            raw_markdown = append_references(&raw_markdown, &references_path);
        }
        get_html(&raw_markdown)
    } else if fragments.is_some() {
        let mut markdown_without_title = markdown_without_title.to_string();
        if let Some(header) = fragments.and_then(|f| f.get("markdown_header")) {
            markdown_without_title.insert_str(0, format!("{header}\n").as_str());
        }
        if let Some(footer) = fragments.and_then(|f| f.get("markdown_footer")) {
            markdown_without_title.push_str(format!("\n{footer}").as_str());
        }
        if let Some(references) = fragments.and_then(|f| f.get("references")) {
            markdown_without_title.push_str(format!("\n\n{references}").as_str());
        }
        get_html(&markdown_without_title)
    } else {
        get_html(&markdown_without_title)
    };

    let description = get_description(&frontmatter);
    let tags = get_tags(&frontmatter);
    let slug = get_slug(&frontmatter, path);
    let date = get_date(&frontmatter, path);
    let extra = frontmatter.get("extra").map(std::borrow::ToOwned::to_owned);
    let links_to = get_links_to(&html);
    let back_links = Vec::new(); // will be mutated later
    let card_image = get_card_image(&frontmatter, &html, path, &slug);
    let banner_image = get_banner_image(&frontmatter, path, &slug);
    let authors = get_authors(&frontmatter, Some(site.default_author.clone()));
    let pinned = frontmatter
        .get("pinned")
        .map_or(false, |p| p.as_bool().unwrap_or(false));

    let stream = if date.is_some() {
        get_stream(&frontmatter)
    } else {
        None
    };

    let content = Content {
        title,
        description,
        slug,
        html,
        tags,
        date,
        extra,
        links_to,
        back_links,
        card_image,
        banner_image,
        authors,
        stream,
        pinned,
    };
    Ok(content)
}

/// Capture `card_image` from frontmatter, then if not defined
/// take the first img src found in the post content
fn get_card_image(
    frontmatter: &Frontmatter,
    html: &str,
    path: &Path,
    slug: &str,
) -> Option<String> {
    if let Some(card_image) = frontmatter.get("card_image") {
        return Some(card_image.to_string());
    }

    // Try to find image matching the slug
    if let Some(value) = find_matching_file(slug, path, "card", &["png", "jpg", "jpeg"]) {
        return Some(value);
    }

    // try banner_image
    if let Some(banner_image) = get_banner_image(frontmatter, path, slug) {
        return Some(banner_image);
    }

    // first <img> src attribute
    let img_regex = Regex::new(r#"<img[^>]*src="([^"]+)""#).unwrap();
    img_regex
        .captures(html)
        .and_then(|caps| caps.get(1).map(|m| m.as_str().to_string()))
}

fn find_matching_file(slug: &str, path: &Path, kind: &str, exts: &[&str]) -> Option<String> {
    // check if a file named {slug}.card.{png,jpg,jpeg} exists in the same directory
    for ext in exts {
        let image_filename = format!("{slug}.{kind}.{ext}");
        let mut path = path.to_path_buf();
        path.pop();
        path.push("media");
        path.push(&image_filename);
        if path.exists() {
            return Some(format!("media/{image_filename}"));
        }
    }
    None
}

fn get_banner_image(frontmatter: &Frontmatter, path: &Path, slug: &str) -> Option<String> {
    if let Some(banner_image) = frontmatter.get("banner_image") {
        return Some(banner_image.as_str().unwrap().trim_matches('"').to_string());
    }

    // Try to find image matching the slug
    if let Some(value) = find_matching_file(slug, path, "banner", &["png", "jpg", "jpeg"]) {
        return Some(value);
    }

    // attempt to get extra.banner_image
    if let Some(extra) = frontmatter.get("extra") {
        if let Some(extra) = extra.as_object() {
            if let Some(banner_image) = extra.get("banner_image") {
                let url = banner_image.to_string();
                // trim start and end quotes
                return Some(url.trim_matches('"').to_string());
            }
        }
    }
    None
}

/// Extract all the internal links from the html content
/// that point to a internal .html file (excluding http links)
/// and return them as a vector of strings
fn get_links_to(html: &str) -> Option<Vec<String>> {
    let mut result = Vec::new();
    let re = Regex::new(r#"href="([^"]+)\.html""#).unwrap();
    for cap in re.captures_iter(html) {
        if let Some(m) = cap.get(1) {
            let href = m.as_str();
            if !href.starts_with("http") {
                result.push(href.trim_start_matches("./").to_string());
            }
        }
    }
    if result.is_empty() {
        return None;
    }
    Some(result)
}

/// Convert markdown to html using comrak
pub fn get_html(markdown: &str) -> String {
    let mut options = ComrakOptions::default();
    options.render.unsafe_ = true;
    options.render.ignore_empty_links = true;
    options.render.figure_with_caption = true;
    options.parse.relaxed_tasklist_matching = true;
    // options.parse.broken_link_callback = TODO: implement this to warn about broken links
    // options.extension.image_url_rewriter = TODO: implement this to point to small image and have a link to the full image
    options.extension.tagfilter = false;
    options.extension.strikethrough = true;
    options.extension.table = true;
    options.extension.autolink = true;
    options.extension.tasklist = true;
    options.extension.footnotes = true;
    options.extension.description_lists = true;
    options.extension.multiline_block_quotes = true;
    options.extension.underline = true;
    options.extension.spoiler = true;
    options.extension.greentext = true;
    options.extension.shortcodes = true;
    options.extension.header_ids = Some("toc-".to_string());
    options.extension.wikilinks_title_before_pipe = true;
    // options.extension.link_url_rewriter = TODO: implement this to replace fix_internal_links

    fix_internal_links(&markdown_to_html(markdown, &options))
}

/// Takes the html content, finds all the internal links and
/// fixes them to point to the correct html file
/// Also removes the .md|.html extension from the text of the link
fn fix_internal_links(html: &str) -> String {
    let re = Regex::new(r#"<a[^>]*href="([^"]+)"[^>]*>(.*?)</a>"#).unwrap();
    re.replace_all(html, |caps: &regex::Captures| {
        let href = &caps[1];
        let text = &caps[2];
        let is_internal = !href.starts_with("http");
        let href_ends_in_html = std::path::Path::new(href)
            .extension()
            .map_or(false, |ext| ext.eq_ignore_ascii_case("html"));
        let new_href = if is_internal {
            if let Some(stripped) = href.strip_suffix(".md") {
                format!("{stripped}.html")
            } else if !href_ends_in_html {
                format!("{href}.html")
            } else {
                href.to_string()
            }
        } else {
            href.to_string()
        };

        let text_ends_in_md = std::path::Path::new(text)
            .extension()
            .map_or(false, |ext| ext.eq_ignore_ascii_case("md"));
        let text_ends_in_html = std::path::Path::new(text)
            .extension()
            .map_or(false, |ext| ext.eq_ignore_ascii_case("html"));
        let new_text = if is_internal && text_ends_in_md {
            &text[..text.len() - 3]
        } else if is_internal && text_ends_in_html {
            &text[..text.len() - 5]
        } else {
            text
        };

        format!(r#"<a href="{new_href}">{new_text}</a>"#)
    })
    .to_string()
}

/// Extract the frontmatter from the content
/// If the content does not start with `---` return an empty frontmatter
/// Otherwise extract the frontmatter and the content after the frontmatter
/// and return them as a tuple
/// The content after the frontmatter is the markdown content
/// If the frontmatter is not valid yaml, return an error
fn parse_front_matter(content: &str) -> Result<(Frontmatter, &str), String> {
    // strip leading empty lines from content
    // this is needed because the frontmatter parser does not like leading empty lines
    let content = content.trim_start_matches('\n');
    if content.starts_with("---") {
        extract_fm_content(content).map_err(|e| e.to_string())
    } else {
        Ok((Frontmatter::new(), content))
    }
}

pub fn extract_fm_content(content: &str) -> Result<(Frontmatter, &str), String> {
    let (raw_frontmatter, remaining_content) = extract_raw_frontmatter(content)?;
    let format = detect_format(raw_frontmatter)?;
    let frontmatter = parse(raw_frontmatter, format)?;

    Ok((frontmatter, remaining_content))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_fix_internal_links_with_md_extension() {
        let html = r#"<a href="test.md">test.md</a>"#;
        let expected = r#"<a href="test.html">test</a>"#;
        assert_eq!(fix_internal_links(html), expected);
    }

    #[test]
    fn test_fix_internal_links_with_html_extension() {
        let html = r#"<a href="test.html">test.html</a>"#;
        let expected = r#"<a href="test.html">test</a>"#;
        assert_eq!(fix_internal_links(html), expected);
    }

    #[test]
    fn test_fix_internal_links_without_extension() {
        let html = r#"<a href="test">test</a>"#;
        let expected = r#"<a href="test.html">test</a>"#;
        assert_eq!(fix_internal_links(html), expected);
    }

    #[test]
    fn test_fix_internal_links_external_link() {
        let html = r#"<a href="http://example.com">example</a>"#;
        let expected = r#"<a href="http://example.com">example</a>"#;
        assert_eq!(fix_internal_links(html), expected);
    }

    #[test]
    fn test_fix_internal_links_mixed_content() {
        let html = r#"<a href="test.md">test.md</a> and <a href="http://example.com">example</a>"#;
        let expected =
            r#"<a href="test.html">test</a> and <a href="http://example.com">example</a>"#;
        assert_eq!(fix_internal_links(html), expected);
    }

    #[test]
    fn test_get_links_to_with_internal_links() {
        let html = r#"<a href="./test1.html">test1</a> <a href="./test2.html">test2</a>"#;
        let expected = Some(vec!["test1".to_string(), "test2".to_string()]);
        assert_eq!(get_links_to(html), expected);
    }

    #[test]
    fn test_get_links_to_with_internal_links_no_slash() {
        let html = r#"<a href="test1.html">test1</a> <a href="test2.html">test2</a>"#;
        let expected = Some(vec!["test1".to_string(), "test2".to_string()]);
        assert_eq!(get_links_to(html), expected);
    }

    #[test]
    fn test_get_links_to_with_no_internal_links() {
        let html = r#"<a href="http://example.com">example</a>"#;
        let expected: Option<Vec<String>> = None;
        assert_eq!(get_links_to(html), expected);
    }

    #[test]
    fn test_get_links_to_with_mixed_links() {
        let html = r#"<a href="./test1.html">test1</a> <a href="test2.html">test2</a> <a href="http://example.com">example</a>"#;
        let expected = Some(vec!["test1".to_string(), "test2".to_string()]);
        assert_eq!(get_links_to(html), expected);
    }

    #[test]
    fn test_get_links_to_with_no_links() {
        let html = r"<p>No links here</p>";
        let expected: Option<Vec<String>> = None;
        assert_eq!(get_links_to(html), expected);
    }

    #[test]
    fn test_get_links_to_with_empty_string() {
        let html = "";
        let expected: Option<Vec<String>> = None;
        assert_eq!(get_links_to(html), expected);
    }

    #[test]
    fn test_get_html_basic_markdown() {
        let markdown = "# Title\n\nThis is a paragraph.";
        let expected = "<h1><a href=\"#title.html\"></a>Title</h1>\n<p>This is a paragraph.</p>\n";
        assert_eq!(get_html(markdown), expected);
    }

    #[test]
    fn test_get_html_with_links() {
        let markdown = "[example](http://example.com)";
        let expected = "<p><a href=\"http://example.com\">example</a></p>\n";
        assert_eq!(get_html(markdown), expected);
    }

    #[test]
    fn test_get_html_with_internal_links() {
        let markdown = "[internal](./test.md)";
        let expected = "<p><a href=\"./test.html\">internal</a></p>\n";
        assert_eq!(get_html(markdown), expected);
    }

    #[test]
    fn test_get_html_with_internal_links_no_slash() {
        let markdown = "[internal](test.md)";
        let expected = "<p><a href=\"test.html\">internal</a></p>\n";
        assert_eq!(get_html(markdown), expected);
    }

    #[test]
    fn test_get_html_with_images() {
        let markdown = "![alt text](media/image.jpg)";
        let expected = "<p><figure><img src=\"media/image.jpg\" alt=\"alt text\" /></figure></p>\n";
        assert_eq!(get_html(markdown), expected);
    }

    #[test]
    fn test_get_html_with_code_block() {
        let markdown = "```\nlet x = 1;\n```";
        let expected = "<pre><code>let x = 1;\n</code></pre>\n";
        assert_eq!(get_html(markdown), expected);
    }

    #[test]
    fn test_get_html_with_task_list() {
        let markdown = "- [x] Task 1\n- [ ] Task 2";
        let expected = "<ul>\n<li><input type=\"checkbox\" checked=\"\" disabled=\"\" /> Task 1</li>\n<li><input type=\"checkbox\" disabled=\"\" /> Task 2</li>\n</ul>\n";
        assert_eq!(get_html(markdown), expected);
    }

    #[test]
    fn test_get_html_with_table() {
        let markdown = "| Header1 | Header2 |\n| ------- | ------- |\n| Cell1   | Cell2   |";
        let expected = "<table>\n<thead>\n<tr>\n<th>Header1</th>\n<th>Header2</th>\n</tr>\n</thead>\n<tbody>\n<tr>\n<td>Cell1</td>\n<td>Cell2</td>\n</tr>\n</tbody>\n</table>\n";
        assert_eq!(get_html(markdown), expected);
    }

    #[test]
    fn test_get_card_image_from_frontmatter() {
        let mut frontmatter = Frontmatter::new();
        frontmatter.insert(
            "card_image".to_string(),
            frontmatter_gen::Value::String("media/image.jpg".to_string()),
        );
        let html = r#"<p>Some content</p><img src="media/other.jpg" />"#;
        let expected = Some("\"media/image.jpg\"".to_string());
        // assert_eq!(get_card_image(&frontmatter, html, ), expected);
        assert_eq!(
            get_card_image(&frontmatter, html, Path::new("test"), "test"),
            expected
        );
    }

    #[test]
    fn test_get_card_image_from_html() {
        let frontmatter = Frontmatter::new();
        let html = r#"<p>Some content</p><img src="media/image.jpg" />"#;
        let expected = Some("media/image.jpg".to_string());
        assert_eq!(
            get_card_image(&frontmatter, html, Path::new("test"), "test"),
            expected
        );
    }

    #[test]
    fn test_get_card_image_no_image() {
        let frontmatter = Frontmatter::new();
        let html = "<p>Some content</p>";
        let expected: Option<String> = None;
        assert_eq!(
            get_card_image(&frontmatter, html, Path::new("test"), "test"),
            expected
        );
    }

    #[test]
    fn test_get_card_image_with_multiple_images() {
        let frontmatter = Frontmatter::new();
        let html = r#"<p>Some content</p><img src="image1.jpg" /><img src="image2.jpg" />"#;
        let expected = Some("image1.jpg".to_string());
        assert_eq!(
            get_card_image(&frontmatter, html, Path::new("test"), "test"),
            expected
        );
    }

    #[test]
    fn test_get_card_image_with_invalid_html() {
        let frontmatter = Frontmatter::new();
        let html = r#"<p>Some content</p><img src="image.jpg"#;
        let expected: Option<String> = None;
        assert_eq!(
            get_card_image(&frontmatter, html, Path::new("test"), "test"),
            expected
        );
    }

    #[test]
    fn test_get_content_with_valid_frontmatter() {
        let path = Path::new("test_get_content_with_valid_frontmatter.md");
        let content = r#"
---
title: Test Title
description: "Test Description"
tags: ["tag1", "tag2"]
slug: "test-title"
date: "2023-01-01"
---
# Test Content
This is a test content.
"#;
        fs::write(path, content).unwrap();
        let result = get_content(path, None, &Marmite::default()).unwrap();
        assert_eq!(result.title, "Test Title");
        assert_eq!(result.description, Some("\"Test Description\"".to_string()));
        assert_eq!(result.slug, "test-title");
        assert_eq!(result.tags, vec!["tag1".to_string(), "tag2".to_string()]);
        assert_eq!(result.date.unwrap().to_string(), "2023-01-01 00:00:00");
        assert_eq!(result.html, "<h1><a href=\"#test-content.html\"></a>Test Content</h1>\n<p>This is a test content.</p>\n");
        fs::remove_file(path).unwrap();
    }

    #[test]
    fn test_get_content_with_invalid_frontmatter() {
        let path = Path::new("test_get_content_with_invalid_frontmatter.md");
        let content = r#"
---
title: "Test Title"
description: "Test Description"
tags: ["tag1", "tag2"
slug: "test-title"
date: "2023-01-01"
extra: "extra content"
---
# Test Content
This is a test content.
"#;
        fs::write(path, content).unwrap();
        let result = get_content(path, None, &Marmite::default());
        assert!(result.is_err());
        fs::remove_file(path).unwrap();
    }

    #[test]
    fn test_get_content_without_frontmatter() {
        let path = Path::new("test_get_content_without_frontmatter.md");
        let content = r"
# Test Content
This is a test content.
";
        fs::write(path, content).unwrap();
        let result = get_content(path, None, &Marmite::default()).unwrap();
        assert_eq!(result.title, "Test Content".to_string());
        assert_eq!(result.description, None);
        assert_eq!(result.slug, "test_get_content_without_frontmatter");
        assert!(result.tags.is_empty());
        assert!(result.date.is_none());
        assert!(result.extra.is_none());
        assert_eq!(result.html, "<p>This is a test content.</p>\n");
        fs::remove_file(path).unwrap();
    }

    #[test]
    fn test_get_content_with_empty_file() {
        let path = Path::new("test_get_content_with_empty_file.md");
        let content = "";
        fs::write(path, content).unwrap();
        let result = get_content(path, None, &Marmite::default()).unwrap();
        assert_eq!(result.slug, "test_get_content_with_empty_file".to_string());
        fs::remove_file(path).unwrap();
    }
}
