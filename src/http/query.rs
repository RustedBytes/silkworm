use std::borrow::Cow;

use url::Url;

use crate::errors::{SilkwormError, SilkwormResult};
use crate::types::Params;

pub(crate) fn build_url_with_params(url: &str, params: &Params) -> SilkwormResult<String> {
    if params.is_empty() {
        let mut parsed = Url::parse(url)
            .map_err(|err| SilkwormError::Http(format!("Invalid URL {url}: {err}")))?;
        parsed.set_fragment(None);
        return Ok(parsed.to_string());
    }
    let mut parsed =
        Url::parse(url).map_err(|err| SilkwormError::Http(format!("Invalid URL {url}: {err}")))?;
    parsed.set_fragment(None);

    let merged = merged_query_pairs(&parsed, params);

    parsed.query_pairs_mut().clear();
    {
        let mut pairs = parsed.query_pairs_mut();
        for (key, value) in merged {
            pairs.append_pair(&key, &value);
        }
    }

    Ok(parsed.to_string())
}

pub(crate) fn canonical_url_with_params(url: &str, params: &Params) -> SilkwormResult<String> {
    let mut parsed =
        Url::parse(url).map_err(|err| SilkwormError::Http(format!("Invalid URL {url}: {err}")))?;
    parsed.set_fragment(None);
    let mut merged = merged_query_pairs(&parsed, params);
    merged.sort();

    parsed.query_pairs_mut().clear();
    {
        let mut pairs = parsed.query_pairs_mut();
        for (key, value) in merged {
            pairs.append_pair(&key, &value);
        }
    }

    Ok(parsed.to_string())
}

fn merged_query_pairs<'a>(url: &Url, params: &'a Params) -> Vec<(Cow<'a, str>, Cow<'a, str>)> {
    let mut merged: Vec<(Cow<'a, str>, Cow<'a, str>)> = url
        .query_pairs()
        .map(|(k, v)| (Cow::Owned(k.into_owned()), Cow::Owned(v.into_owned())))
        .collect();

    if params.is_empty() {
        return merged;
    }

    merged.retain(|(key, _)| !params.contains_key(key.as_ref()));

    // URL pairs must be owned before mutating the parsed URL. Request params
    // remain borrowed through serialization; no public result borrows them.
    // HashMap iteration order is nondeterministic; keep appended params stable.
    let mut additions: Vec<_> = params
        .iter()
        .map(|(key, value)| (Cow::Borrowed(key.as_str()), Cow::Borrowed(value.as_str())))
        .collect();
    additions.sort();
    merged.extend(additions);

    merged
}
#[cfg(test)]
mod tests {
    use super::*;

    // Frozen pre-optimization algorithm for differential serialization checks.
    fn owned_reference(url: &str, params: &Params, canonical: bool) -> SilkwormResult<String> {
        let mut parsed = Url::parse(url)
            .map_err(|err| SilkwormError::Http(format!("Invalid URL {url}: {err}")))?;
        parsed.set_fragment(None);
        if params.is_empty() && !canonical {
            return Ok(parsed.to_string());
        }
        let mut pairs: Vec<(String, String)> = parsed
            .query_pairs()
            .map(|(key, value)| (key.into_owned(), value.into_owned()))
            .collect();
        if !params.is_empty() {
            pairs.retain(|(key, _)| !params.contains_key(key));
            let mut additions: Vec<_> = params
                .iter()
                .map(|(key, value)| (key.clone(), value.clone()))
                .collect();
            additions.sort();
            pairs.extend(additions);
        }
        if canonical {
            pairs.sort();
        }
        parsed.query_pairs_mut().clear();
        for (key, value) in pairs {
            parsed.query_pairs_mut().append_pair(&key, &value);
        }
        Ok(parsed.to_string())
    }

    #[test]
    fn borrowed_parameters_match_previous_owned_serialization() {
        for url in [
            "https://EXAMPLE.com/path?z=2&x=1&x=0#frag",
            "https://example.com/?q=%FF&same=a&same=b&=old&encoded%20key=%E9",
            "https://example.com/",
            "https://user:pass@example.com:443/a%20b?x=%2B+%26&x=#f",
            "ftp://example.com/?a=b#frag",
            "http://[::1]/?a=1",
            "not a URL",
            "https://[bad host]/",
        ] {
            for count in [0, 1, 8, 64] {
                let params: Params = (0..count)
                    .map(|i| {
                        let key = match i {
                            0 => String::new(),
                            1 => "x".into(),
                            2 => "same".into(),
                            3 => "encoded key".into(),
                            _ => format!("ключ-{i}"),
                        };
                        (key, format!("ї / + & = ? 雪 {i}"))
                    })
                    .collect();
                let original = params.clone();
                for canonical in [false, true] {
                    let actual = if canonical {
                        canonical_url_with_params(url, &params)
                    } else {
                        build_url_with_params(url, &params)
                    };
                    let expected = owned_reference(url, &params, canonical);
                    assert_eq!(
                        actual.map_err(|err| err.to_string()),
                        expected.map_err(|err| err.to_string()),
                        "url={url}, count={count}, canonical={canonical}",
                    );
                }
                assert_eq!(params, original);
            }
        }
    }

    #[test]
    fn params_override_all_duplicates_without_reordering_other_url_pairs() {
        let params = Params::from([("a".into(), "new".into()), (String::new(), String::new())]);
        let url = "https://example.com/?b=2&a=old&b=1&a=older&=old#frag";
        assert_eq!(
            build_url_with_params(url, &params).unwrap(),
            "https://example.com/?b=2&b=1&=&a=new"
        );
        assert_eq!(
            canonical_url_with_params(url, &params).unwrap(),
            "https://example.com/?=&a=new&b=1&b=2"
        );
    }

    #[test]
    fn returned_url_outlives_request_params_and_errors_keep_http_category() {
        let url = {
            let params = Params::from([("q".into(), "їжак".into())]);
            build_url_with_params("https://example.com/", &params).unwrap()
        };
        assert_eq!(url, "https://example.com/?q=%D1%97%D0%B6%D0%B0%D0%BA");
        assert!(matches!(
            build_url_with_params("invalid", &Params::new()),
            Err(SilkwormError::Http(_))
        ));
    }
}
