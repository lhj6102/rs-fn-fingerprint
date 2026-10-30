use rs_fn_fingerprint::{Error, Fingerprints, combine, fingerprint_rust, fingerprint_toml};

const RUST: &str = include_str!("fixtures/items.rs");
const TOML: &str = include_str!("fixtures/tables.toml");
const RUST_QUERIES: &[&str] = &[
    "fn:selected",
    "fn:separate",
    "arm:ops:1000",
    "arm:ops:1100",
    "arm:ops:1200",
    "arm:ops:9",
    "arm:text:hello",
    "fn:method_only",
];

fn changed(before: &Fingerprints, after: &Fingerprints) -> Vec<String> {
    assert_eq!(
        before.keys().collect::<Vec<_>>(),
        after.keys().collect::<Vec<_>>()
    );
    before
        .iter()
        .filter(|(key, value)| after.get(*key) != Some(*value))
        .map(|(key, _)| key.clone())
        .collect()
}

#[test]
fn rust_format_comments_and_docs_are_irrelevant() {
    let original = fingerprint_rust(RUST, RUST_QUERIES, false).unwrap();
    let formatted = fingerprint_rust(
        include_str!("fixtures/items_formatted.rs"),
        RUST_QUERIES,
        false,
    )
    .unwrap();
    assert_eq!(original, formatted);
}

#[test]
fn body_changes_only_its_function() {
    let before = fingerprint_rust(RUST, RUST_QUERIES, false).unwrap();
    let after = fingerprint_rust(
        &RUST.replace("helper() + OFFSET", "helper() - OFFSET"),
        RUST_QUERIES,
        false,
    )
    .unwrap();
    assert_eq!(changed(&before, &after), ["crate::selected"]);
}

#[test]
fn helper_changes_only_its_direct_users() {
    let before = fingerprint_rust(RUST, RUST_QUERIES, false).unwrap();
    let after = fingerprint_rust(
        &RUST.replace("leaf() + SCALE", "leaf() - SCALE"),
        RUST_QUERIES,
        false,
    )
    .unwrap();
    assert_eq!(
        changed(&before, &after),
        ["crate::selected", "ops:1000", "ops:1100", "text:hello"]
    );
}

#[test]
fn helpers_are_only_one_level_deep() {
    let before = fingerprint_rust(RUST, RUST_QUERIES, false).unwrap();
    let after = fingerprint_rust(&RUST.replace("{ 7 }", "{ 70 }"), RUST_QUERIES, false).unwrap();
    assert_eq!(before, after);
    let direct = ["fn:helper"];
    assert_ne!(
        fingerprint_rust(RUST, &direct, false).unwrap(),
        fingerprint_rust(&RUST.replace("{ 7 }", "{ 70 }"), &direct, false).unwrap()
    );
}

#[test]
fn constants_and_statics_are_direct_helpers() {
    let before = fingerprint_rust(RUST, RUST_QUERIES, false).unwrap();
    let after = fingerprint_rust(
        &RUST.replace("static OFFSET: i32 = 3", "static OFFSET: i32 = 30"),
        RUST_QUERIES,
        false,
    )
    .unwrap();
    assert_eq!(changed(&before, &after), ["crate::selected", "ops:1200"]);
    let after = fingerprint_rust(
        &RUST.replace("const SCALE: i32 = 2", "const SCALE: i32 = 20"),
        RUST_QUERIES,
        false,
    )
    .unwrap();
    assert_eq!(changed(&before, &after), ["ops:1200"]);
}

#[test]
fn or_patterns_and_guards_select_complete_arms() {
    let before = fingerprint_rust(RUST, RUST_QUERIES, false).unwrap();
    assert_eq!(before["ops:1000"], before["ops:1100"]);
    let after = fingerprint_rust(
        &RUST.replace("if enabled", "if !enabled"),
        RUST_QUERIES,
        false,
    )
    .unwrap();
    assert_eq!(changed(&before, &after), ["ops:1200"]);
    assert!(matches!(
        fingerprint_rust(RUST, &["arm:ops:900"], false),
        Err(Error::Missing(_))
    ));
}

#[test]
fn nested_modules_impls_and_collisions() {
    let results = fingerprint_rust(RUST, &["fn:*"], false).unwrap();
    for key in [
        "crate::selected",
        "nested::selected",
        "Worker::selected",
        "nested_only",
        "method_only",
    ] {
        assert!(results.contains_key(key), "missing {key}");
    }
    let source = "struct A; trait T {} impl A { fn same() {} } impl T for A { fn same() {} }";
    let results = fingerprint_rust(source, &["fn:same"], false).unwrap();
    assert!(results.contains_key("A::same"));
    assert!(results.contains_key("<A as T>::same"));
}

#[test]
fn duplicate_impl_alternatives_do_not_overwrite() {
    let source = "struct A; impl A { fn f() { 1; } } impl A { fn f() { 2; } }";
    let results = fingerprint_rust(source, &["fn:f"], false).unwrap();
    assert_eq!(results.len(), 2);
    assert!(results.contains_key("A::f#2"));
}

#[test]
fn missing_rust_queries_and_invalid_syntax() {
    assert!(matches!(
        fingerprint_rust(RUST, &["fn:absent"], false),
        Err(Error::Missing(_))
    ));
    let results = fingerprint_rust(RUST, &["fn:absent", "optional=arm:ops:123"], true).unwrap();
    assert_eq!(results["fn:absent"], None);
    assert_eq!(results["optional"], None);
    for query in ["bad", "fn:", "fn:[", "arm:ops", "arm:ops:", "=fn:*", "x="] {
        assert!(fingerprint_rust(RUST, &[query], true).is_err(), "{query}");
    }
    assert!(fingerprint_rust("fn {", &["fn:*"], true).is_err());
}

#[test]
fn aliases_aggregate_and_conflicts_are_explicit() {
    let result = fingerprint_rust(RUST, &["all=fn:*"], false).unwrap();
    assert_eq!(result.len(), 1);
    assert!(result["all"].is_some());
    assert!(matches!(
        fingerprint_rust(RUST, &["one=fn:helper", "one=fn:leaf"], false),
        Err(Error::Collision(_))
    ));
    assert_eq!(
        fingerprint_rust(RUST, &["fn:helper"], false).unwrap(),
        fingerprint_rust(RUST, &["fn:helper", "fn:helper"], false).unwrap()
    );
}

#[test]
fn literals_can_be_strings_raw_strings_hex_and_suffixed_integers() {
    for query in [
        "arm:text:hello",
        "arm:text:\"hello\"",
        "arm:text:r#\"hello\"#",
    ] {
        assert!(
            fingerprint_rust(RUST, &[query], false)
                .unwrap()
                .contains_key("text:hello")
        );
    }
    assert!(
        fingerprint_rust(RUST, &["arm:text:\"a:b=c\""], false)
            .unwrap()
            .contains_key("text:a:b=c")
    );
    let source = "fn f(n: u32) { match n { 0x10_u32 => (), _ => () } }";
    assert_eq!(
        fingerprint_rust(source, &["arm:f:16"], false).unwrap(),
        fingerprint_rust(source, &["arm:f:0x10"], false).unwrap()
    );
}

#[test]
fn all_matching_arms_including_nested_matches_are_hashed() {
    let source = "fn f(n: u32) { match n { 1 => { match n { 7 => (), _ => () } }, _ => () }; match n { 7 => (), _ => () } }";
    let before = fingerprint_rust(source, &["arm:f:7"], false).unwrap();
    let after = fingerprint_rust(
        &source.replacen("7 => ()", "7 => changed()", 1),
        &["arm:f:7"],
        false,
    )
    .unwrap();
    assert_ne!(before, after);
    assert_eq!(before.len(), 1);
}

#[test]
fn qualified_paths_and_macro_names_are_not_helpers_but_macro_inputs_are() {
    let source = "fn helper() { 1; } fn f() { helper!(); m!(module::helper(), object.helper(), $helper); module::helper(); }";
    assert_eq!(
        fingerprint_rust(source, &["fn:f"], false).unwrap(),
        fingerprint_rust(&source.replace("{ 1; }", "{ 2; }"), &["fn:f"], false).unwrap()
    );
    let changed_macro = source.replace(
        "m!(module::helper(), object.helper(), $helper)",
        "m!(helper())",
    );
    assert_ne!(
        fingerprint_rust(source, &["fn:f"], false).unwrap(),
        fingerprint_rust(&changed_macro, &["fn:f"], false).unwrap()
    );
}

#[test]
fn doc_attributes_nested_in_items_are_stripped_but_other_attributes_remain() {
    let source = "#[inline] fn f() { #[doc = \"a\"] const N: u32 = 1; }";
    let before = fingerprint_rust(source, &["fn:f"], false).unwrap();
    assert_eq!(
        before,
        fingerprint_rust(&source.replace("\"a\"", "\"b\""), &["fn:f"], false).unwrap()
    );
    assert_ne!(
        before,
        fingerprint_rust(&source.replace("#[inline]", "#[cold]"), &["fn:f"], false).unwrap()
    );
}

#[test]
fn toml_formatting_comments_and_key_order_are_irrelevant() {
    let queries = ["settings.100", "variants.100.*", "workers"];
    assert_eq!(
        fingerprint_toml(TOML, &queries).unwrap(),
        fingerprint_toml(include_str!("fixtures/tables_formatted.toml"), &queries).unwrap()
    );
}

#[test]
fn toml_value_edit_changes_exactly_the_affected_query() {
    let queries = [
        "settings.100",
        "settings.200",
        "variants.100.*",
        "missing.*",
    ];
    let before = fingerprint_toml(TOML, &queries).unwrap();
    let after = fingerprint_toml(&TOML.replace("amount = 2", "amount = 20"), &queries).unwrap();
    assert_eq!(changed(&before, &after), ["settings.100"]);
}

#[test]
fn toml_absence_is_a_stable_hash_but_presence_changes_it() {
    let before = fingerprint_toml(TOML, &["extra=new.*", "also=absent.*"]).unwrap();
    assert_eq!(before["extra"], before["also"]);
    assert!(before["extra"].as_ref().unwrap().starts_with("sha256:"));
    let after =
        fingerprint_toml(&format!("{TOML}\n[new.first]\nx = 1\n"), &["extra=new.*"]).unwrap();
    assert_ne!(before["extra"], after["extra"]);
}

#[test]
fn toml_implicit_inline_tables_arrays_and_quoted_keys_are_stable() {
    let before = "[a.b]\nx = { y = 1, z = 2 }\n[\"a.b\"]\ny = 9\n";
    let after = "a.b.x = { z=2, y=1 }\n\"a.b\".y=9\n";
    assert_eq!(
        fingerprint_toml(before, &["*"]).unwrap(),
        fingerprint_toml(after, &["*"]).unwrap()
    );
    assert_ne!(
        fingerprint_toml(before, &["k=a.b"]).unwrap(),
        fingerprint_toml(before, &["k=\"a.b\""]).unwrap()
    );
    assert_ne!(
        fingerprint_toml(TOML, &["workers"]).unwrap(),
        fingerprint_toml(
            &TOML.replace("name = \"two\"", "name = \"three\""),
            &["workers"]
        )
        .unwrap()
    );
}

#[test]
fn toml_non_finite_floats_and_dates_have_distinct_canonical_values() {
    let values = ["inf", "-inf", "nan", "2000-01-01", "\"2000-01-01\""];
    let hashes: std::collections::BTreeSet<_> = values
        .iter()
        .map(|value| fingerprint_toml(&format!("[a]\nx = {value}"), &["a"]).unwrap()["a"].clone())
        .collect();
    assert_eq!(hashes.len(), values.len());
}

#[test]
fn invalid_toml_and_globs_are_errors() {
    assert!(fingerprint_toml("x = [", &["a"]).is_err());
    for query in ["[", "=a", "a=", ""] {
        assert!(fingerprint_toml(TOML, &[query]).is_err());
    }
}

#[test]
fn combined_output_is_order_independent_and_covers_keys() {
    let first = fingerprint_rust(RUST, &["fn:helper", "fn:leaf"], false).unwrap();
    let second = fingerprint_rust(RUST, &["fn:leaf", "fn:helper"], false).unwrap();
    assert_eq!(combine(&first), combine(&second));
    let renamed = fingerprint_rust(RUST, &["renamed=fn:helper", "fn:leaf"], false).unwrap();
    assert_ne!(combine(&first), combine(&renamed));
}

#[test]
fn a_single_result_alias_only_renames_the_key() {
    let normal = fingerprint_rust(RUST, &["fn:helper", "arm:ops:1200"], false).unwrap();
    let renamed = fingerprint_rust(RUST, &["h=fn:helper", "a=arm:ops:1200"], false).unwrap();
    assert_eq!(normal["helper"], renamed["h"]);
    assert_eq!(normal["ops:1200"], renamed["a"]);
    let normal = fingerprint_toml(TOML, &["settings.100"]).unwrap();
    let renamed = fingerprint_toml(TOML, &["s=settings.100"]).unwrap();
    assert_eq!(normal["settings.100"], renamed["s"]);
}

#[test]
fn signed_integer_literals_are_distinct_and_support_all_rust_magnitudes() {
    let source = "fn f(n: i128) { match n { -16 => (), 16 => (), -170141183460469231731687303715884105728 => (), _ => () } }";
    let result = fingerprint_rust(
        source,
        &[
            "arm:f:-16",
            "arm:f:16",
            "arm:f:-170141183460469231731687303715884105728",
        ],
        false,
    )
    .unwrap();
    assert_eq!(result.len(), 3);
    assert_ne!(result["f:-16"], result["f:16"]);
    assert_eq!(
        fingerprint_rust(source, &["arm:f:-16"], false).unwrap(),
        fingerprint_rust(source, &["arm:f:-0x10_i128"], false).unwrap()
    );
}

#[test]
fn doc_comments_inside_macros_are_ignored_without_expanding_them() {
    let source =
        "fn f() { m! { #[doc = \"outer\"] fn g() { m! { #[doc = \"inner\"] struct T; } } } }";
    assert_eq!(
        fingerprint_rust(source, &["fn:f"], false).unwrap(),
        fingerprint_rust(
            &source
                .replace("outer", "new outer")
                .replace("inner", "new inner"),
            &["fn:f"],
            false
        )
        .unwrap()
    );
    let non_doc = source.replace("#[doc = \"outer\"]", "#[inline]");
    assert_ne!(
        fingerprint_rust(source, &["fn:f"], false).unwrap(),
        fingerprint_rust(&non_doc, &["fn:f"], false).unwrap()
    );
}

#[test]
fn helper_item_order_and_changes_in_external_files_are_irrelevant() {
    let before = "mod external; fn a() { b(); c(); } fn b() {} fn c() {}";
    let after = "mod different; fn c() {} fn a() { b(); c(); } fn b() {}";
    assert_eq!(
        fingerprint_rust(before, &["fn:a"], false).unwrap(),
        fingerprint_rust(after, &["fn:a"], false).unwrap()
    );
}

#[test]
fn qself_paths_and_method_calls_do_not_resolve_as_bare_helpers() {
    let source = "fn helper() { 1; } fn f() { <Type>::helper(); <Type as Trait>::helper(); value.helper(); }";
    assert_eq!(
        fingerprint_rust(source, &["fn:f"], false).unwrap(),
        fingerprint_rust(&source.replace("{ 1; }", "{ 2; }"), &["fn:f"], false).unwrap()
    );
}

#[test]
fn block_local_functions_constants_and_statics_are_arm_helpers() {
    let source = "fn f(n: u32) { const C: u32 = 1; static S: u32 = 2; fn h() -> u32 { 3 } match n { 7 => C + S + h(), _ => 0 } }";
    let before = fingerprint_rust(source, &["arm:f:7"], false).unwrap();
    for edited in [
        source.replace("= 1", "= 10"),
        source.replace("= 2", "= 20"),
        source.replace("{ 3 }", "{ 30 }"),
    ] {
        assert_ne!(
            before,
            fingerprint_rust(&edited, &["arm:f:7"], false).unwrap()
        );
    }
    assert!(matches!(
        fingerprint_rust(source, &["fn:h"], false),
        Err(Error::Missing(_))
    ));
}

#[test]
fn equals_inside_quoted_toml_keys_is_not_an_alias() {
    let source = "[\"a=b\"]\nx = 1\n";
    let normal = fingerprint_toml(source, &["\"a=b\""]).unwrap();
    let alias = fingerprint_toml(source, &["named=\"a=b\""]).unwrap();
    let absent = fingerprint_toml("", &["\"a=b\""]).unwrap();
    assert_eq!(normal["\"a=b\""], alias["named"]);
    assert_ne!(normal, absent);
}

#[test]
fn raw_identifiers_use_their_logical_name_for_queries_and_helpers() {
    let source = "fn r#helper() { 1; } fn r#type() { helper(); }";
    let before = fingerprint_rust(source, &["fn:type", "fn:helper"], false).unwrap();
    assert!(before.contains_key("type"));
    assert!(before.contains_key("helper"));
    let after = fingerprint_rust(
        &source.replace("{ 1; }", "{ 2; }"),
        &["fn:type", "fn:helper"],
        false,
    )
    .unwrap();
    assert_eq!(changed(&before, &after), ["helper", "type"]);
}

#[test]
fn attribute_macros_are_not_direct_helper_references() {
    let source = "fn helper() { 1; } #[helper] fn f() {}";
    assert_eq!(
        fingerprint_rust(source, &["fn:f"], false).unwrap(),
        fingerprint_rust(&source.replace("{ 1; }", "{ 2; }"), &["fn:f"], false).unwrap()
    );
}

#[test]
fn literal_branches_cover_if_else_match_and_macro_conditions_only() {
    let source = r#"
        fn shared() -> u32 { 3 }
        fn ops(id: u32) -> u32 {
            let mut out = 0;
            if matches!(id, 41 | 42) { out += shared(); }
            else if id == 43 { out += 4; }
            else { out += 99; }
            match id { 42 if id > 0 => out += 8, _ => out += 44 }
            if id == 45 { out += 42; }
            out
        }
    "#;
    let queries = [
        "branch:ops:42",
        "branch:ops:43",
        "branch:ops:45",
        "branch:ops:999",
    ];
    let before = fingerprint_rust(source, &queries, false).unwrap();
    assert!(before.values().all(Option::is_some));
    let changed_branch =
        fingerprint_rust(&source.replace("out += 4;", "out += 5;"), &queries, false).unwrap();
    assert_eq!(changed(&before, &changed_branch), ["ops:43"]);
    let changed_body =
        fingerprint_rust(&source.replace("out += 42;", "out += 40;"), &queries, false).unwrap();
    assert_eq!(changed(&before, &changed_body), ["ops:45"]);
    let changed_helper =
        fingerprint_rust(&source.replace("{ 3 }", "{ 7 }"), &queries, false).unwrap();
    assert_eq!(changed(&before, &changed_helper), ["ops:42"]);
    assert_eq!(
        before,
        fingerprint_rust(&source.replace("out += 99", "out += 100"), &queries, false).unwrap()
    );
    assert_eq!(
        before,
        fingerprint_rust(
            &source.replace("out += 8", "/* comment */ out += 8"),
            &queries,
            false
        )
        .unwrap()
    );
    assert_ne!(
        before["ops:42"],
        fingerprint_rust(&source.replace("out += 8", "out += 9"), &queries, false).unwrap()["ops:42"]
    );
}

#[test]
fn json_fields_and_paths_select_only_owned_rows_canonically() {
    use rs_fn_fingerprint::fingerprint_json;
    let source = r#"{"nodes":{"41":{"id":41,"values":[1,2]},"42":{"id":42,"value":4}},"rows":[{"id":41,"value":5}]}"#;
    let queries = [
        "field:id:41",
        "path:nodes.41",
        "path:nodes.42",
        "field:id:999",
    ];
    let before = fingerprint_json(source, &queries).unwrap();
    assert!(before.values().all(Option::is_some));
    let reordered = r#"{ "rows": [{"value":5,"id":41}], "nodes": { "42":{"value":4,"id":42}, "41":{"values":[1,2],"id":41} } }"#;
    assert_eq!(before, fingerprint_json(reordered, &queries).unwrap());
    let after = fingerprint_json(&source.replace("\"value\":4", "\"value\":6"), &queries).unwrap();
    assert_eq!(changed(&before, &after), ["path:nodes.42"]);
    let after = fingerprint_json(&source.replace("[1,2]", "[2,1]"), &queries).unwrap();
    assert_eq!(changed(&before, &after), ["field:id:41", "path:nodes.41"]);
    assert_ne!(
        fingerprint_json("{}", &["path:a.b"]).unwrap(),
        fingerprint_json(r#"{"a":{"b":{}}}"#, &["path:a.b"]).unwrap()
    );
    assert_ne!(
        fingerprint_json(r#"{"a.b":{}}"#, &["path:a.b"]).unwrap(),
        fingerprint_json(r#"{"a":{"b":{}}}"#, &["path:a.b"]).unwrap()
    );
    assert!(fingerprint_json(source, &["bad"]).is_err());
    assert!(fingerprint_json("{", &queries).is_err());
}

#[test]
fn helpers_referenced_only_inside_vec_macros_are_one_hop_inputs() {
    let source = "fn leaf() -> i32 { 1 } fn helper() -> i32 { leaf() } fn a() { vec![helper(), helper::<u32>()]; } fn b() { vec![vec![helper()]]; } fn c() { vec![3]; }";
    let queries = ["fn:a", "fn:b", "fn:c"];
    let before = fingerprint_rust(source, &queries, false).unwrap();
    let after = fingerprint_rust(
        &source.replace("{ leaf() }", "{ leaf() + 1 }"),
        &queries,
        false,
    )
    .unwrap();
    assert_eq!(changed(&before, &after), ["a", "b"]);
    assert_eq!(
        before,
        fingerprint_rust(&source.replace("{ 1 }", "{ 2 }"), &queries, false).unwrap()
    );
}
