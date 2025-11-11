use anyhow::Result;
use itertools::Itertools;

pub struct Nav {
    href: String,
    text: String,
}

impl Nav {
    pub fn new<T, U>(href: T, text: U) -> Self
    where
        T: ToString,
        U: ToString,
    {
        Self {
            href: href.to_string(),
            text: text.to_string(),
        }
    }
}

impl<T: ToString> From<[T; 2]> for Nav {
    fn from(value: [T; 2]) -> Self {
        Self {
            href: value[0].to_string(),
            text: value[1].to_string(),
        }
    }
}

impl<T: ToString> From<(T, T)> for Nav {
    fn from(value: (T, T)) -> Self {
        Self {
            href: value.0.to_string(),
            text: value.1.to_string(),
        }
    }
}

pub fn gen_plaintext(str: impl AsRef<str>) -> Result<String> {
    let mut renderenv = minijinja::Environment::new();
    minijinja_embed::load_templates!(&mut renderenv);

    let template = renderenv
        .get_template("text.jinja")
        .expect("Failed loading links template");
    let renderctx = minijinja::context! {
        title => "Text",
        subtitle => "Text here",
        text => str.as_ref()
    };

    Ok(template
        .render(renderctx)
        .expect("Failed to render template"))
}

pub fn gen_linkpage(navs: Vec<Nav>) -> Result<String> {
    let mut renderenv = minijinja::Environment::new();
    minijinja_embed::load_templates!(&mut renderenv);

    let template = renderenv
        .get_template("links.jinja")
        .expect("Failed loading links template");
    let renderctx = minijinja::context! {
        title => "Text",
        subtitle => "Text here",
        navigation => navs
            .iter()
            .map(|x| {
                minijinja::context! {
                    href => format!("playlist/{}", x.href),
                    text => x.text
                }
            })
            .collect_vec()
    };

    Ok(template
        .render(renderctx)
        .expect("Failed to render template"))
}
