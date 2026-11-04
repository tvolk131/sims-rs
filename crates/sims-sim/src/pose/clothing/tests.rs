use super::*;

#[cfg(feature = "private-fixtures")]
#[test]
fn native_file_map_normalization_growth_and_first_values_match() {
    let report: serde_json::Value = serde_json::from_str(include_str!(
        "../../../tests/fixtures/avatar-file-names.json"
    ))
    .unwrap();
    for (case, row) in report["cases"].as_array().unwrap().iter().enumerate() {
        let names: Vec<Vec<u8>> = serde_json::from_value(row["input"].clone()).unwrap();
        let mut map = name_map::Files::default();
        let mut events = vec![];
        for (i, name) in names.iter().enumerate() {
            let inserted = map.insert(name, i).unwrap();
            events.push(serde_json::json!([
                name_map::Files::<usize>::key(name).unwrap(),
                inserted,
                map.len(),
                map.bucket_count(),
                map.get(name).unwrap().unwrap()
            ]));
        }
        let (order, first_indices): (Vec<&[u8]>, Vec<&usize>) = map.iter().unzip();
        assert_eq!(
            serde_json::json!({"events": events, "order": order, "first_indices": first_indices}),
            row["original"],
            "file map {case}"
        );
        let mut mapped = map
            .try_map(|_, &value| Ok::<_, ()>((value % 3 != 0).then_some(value)))
            .unwrap();
        assert_eq!(mapped.bucket_count(), map.bucket_count());
        assert_eq!(mapped.len(), map.len());
        assert_eq!(
            mapped.iter().map(|(name, _)| name).collect::<Vec<_>>(),
            order
        );
        for (name, value) in mapped.iter() {
            let original = *map.get(name).unwrap().unwrap();
            assert_eq!(*value, (original % 3 != 0).then_some(original));
        }
        // Subsequent insertion must also see the same bucket/chains, including
        // histories whose next new entry triggers a rehash.
        map.insert(b"after-projection.ndx", usize::MAX).unwrap();
        mapped
            .insert(b"after-projection.ndx", Some(usize::MAX))
            .unwrap();
        assert_eq!(mapped.bucket_count(), map.bucket_count());
        assert_eq!(
            mapped.iter().map(|(name, _)| name).collect::<Vec<_>>(),
            map.iter().map(|(name, _)| name).collect::<Vec<_>>()
        );
    }
}

#[cfg(feature = "private-fixtures")]
#[test]
fn native_avatar_paths_preserve_expansion_order_and_registration() {
    use catalog_paths::{Host, Kind, Paths};
    struct Services {
        input: serde_json::Value,
        registered: Vec<Vec<u8>>,
        events: Vec<serde_json::Value>,
    }
    fn text(path: &[u8]) -> &str {
        std::str::from_utf8(path).unwrap()
    }
    fn strings(paths: &[Vec<u8>]) -> Vec<&str> {
        paths.iter().map(|p| text(p)).collect()
    }
    impl Host for Services {
        fn exists(&mut self, path: &[u8]) -> Result<bool, String> {
            let found = self.input["exists"]
                .as_array()
                .unwrap()
                .iter()
                .any(|s| s == text(path));
            self.events
                .push(serde_json::json!(["exists", text(path), found]));
            Ok(found)
        }
        fn providers(&mut self) -> Result<Vec<Vec<u8>>, String> {
            self.events.push(serde_json::json!(["providers"]));
            Ok(self.input["providers"]
                .as_array()
                .unwrap()
                .iter()
                .map(|s| s.as_str().unwrap().as_bytes().to_vec())
                .collect())
        }
        fn registered(&mut self, path: &[u8]) -> Result<bool, String> {
            let found = self.registered.iter().any(|p| p == path);
            self.events
                .push(serde_json::json!(["registered", text(path), found]));
            Ok(found)
        }
        fn register(&mut self, path: &[u8], recursive: bool, archives: bool) -> Result<(), String> {
            self.events.push(serde_json::json!([
                "register",
                text(path),
                i32::from(recursive),
                i32::from(archives)
            ]));
            self.registered.push(path.to_vec());
            Ok(())
        }
        fn load(&mut self, kind: Kind, paths: &[Vec<u8>]) -> Result<(), String> {
            let kind = match kind {
                Kind::Appearances => "appearances",
                Kind::Animations => "animations",
            };
            self.events
                .push(serde_json::json!(["load", kind, strings(paths)]));
            Ok(())
        }
    }
    let report: serde_json::Value =
        serde_json::from_str(include_str!("../../../tests/fixtures/avatar-paths.json")).unwrap();
    for (case, row) in report["cases"].as_array().unwrap().iter().enumerate() {
        let input = &row["input"];
        let mut host = Services {
            input: input.clone(),
            registered: input["registered"]
                .as_array()
                .unwrap()
                .iter()
                .map(|s| s.as_str().unwrap().as_bytes().to_vec())
                .collect(),
            // Catalog-object construction is a supplied boundary, not part
            // of this Rust path controller.
            events: vec![serde_json::json!(["construct", true])],
        };
        let mut loaded = None;
        Paths::load_once(
            &mut loaded,
            input["root"].as_str().unwrap().as_bytes(),
            input["flags"].as_i64().unwrap() as i16,
            &mut host,
        )
        .unwrap();
        let paths = loaded.as_ref().unwrap();
        assert_eq!(
            serde_json::json!({ "appearances": strings(&paths.appearances),
            "animations": strings(&paths.animations), "events": host.events }),
            row["original"],
            "path case {case}"
        );
        let before = (loaded.clone(), host.events.clone());
        Paths::load_once(&mut loaded, b"different-root", 0, &mut host).unwrap();
        assert_eq!((loaded, host.events), before, "loaded guard {case}");
    }
    let report: serde_json::Value =
        serde_json::from_str(include_str!("../../../tests/fixtures/texture-paths.json")).unwrap();
    for (case, row) in report["cases"].as_array().unwrap().iter().enumerate() {
        let input = &row["input"];
        let decode = |key: &str| -> Vec<Vec<u8>> {
            input[key]
                .as_array()
                .unwrap()
                .iter()
                .map(|s| s.as_str().unwrap().as_bytes().to_vec())
                .collect()
        };
        let mut host = Services {
            input: input.clone(),
            registered: decode("registered"),
            events: vec![],
        };
        let paths = catalog_paths::texture_paths(
            input["prefix"].as_str().unwrap().as_bytes(),
            input["flags"].as_i64().unwrap() as i16,
            &decode("providers"),
            &mut host,
        )
        .unwrap();
        host.events
            .push(serde_json::json!(["publish", strings(&paths)]));
        assert_eq!(
            serde_json::json!({"paths": strings(&paths), "events": host.events}),
            row["original"],
            "texture path case {case}"
        );
    }
}

#[cfg(feature = "private-fixtures")]
#[test]
fn native_texture_resource_names_retain_counted_extension_and_identity() {
    let report: serde_json::Value = serde_json::from_str(include_str!(
        "../../../tests/fixtures/clothing-resource-names.json"
    ))
    .unwrap();
    for (case, row) in report["cases"].as_array().unwrap().iter().enumerate() {
        let names: Vec<Vec<u8>> = serde_json::from_value(row["input"].clone()).unwrap();
        let records: Vec<_> = names
            .iter()
            .map(|name| resource_name::Name::parse(name).unwrap())
            .collect();
        let expected = &row["original"];
        for (i, name) in records.iter().enumerate() {
            assert_eq!(
                serde_json::json!({ "identity": name.identity, "bytes": name.bytes, "visible": name.visible() }),
                expected["records"][i],
                "name {case}/{i}"
            );
        }
        assert_eq!(
            serde_json::json!(records[0] == records[1]),
            expected["equal"],
            "duplicate {case}"
        );
    }
}

#[cfg(feature = "private-fixtures")]
#[test]
fn native_name_map_insertion_growth_and_iteration_match() {
    let mut report: serde_json::Value = serde_json::from_str(include_str!(
        "../../../tests/fixtures/clothing-name-map.json"
    ))
    .unwrap();
    let other_kinds: serde_json::Value = serde_json::from_str(include_str!(
        "../../../tests/fixtures/avatar-name-maps.json"
    ))
    .unwrap();
    for row in other_kinds["cases"].as_array().unwrap() {
        for kind in ["skeleton", "animation"] {
            report["cases"]
                .as_array_mut()
                .unwrap()
                .push(serde_json::json!({
                    "input": row["input"], "original": row["original"][kind]
                }));
        }
    }
    for (case, row) in report["cases"].as_array().unwrap().iter().enumerate() {
        let names: Vec<Vec<u8>> = serde_json::from_value(row["input"].clone()).unwrap();
        let mut map = name_map::Names::default();
        let mut events = vec![];
        for name in names {
            let inserted = map.insert(name);
            events.push(serde_json::json!([inserted, map.len(), map.bucket_count()]));
        }
        assert_eq!(
            serde_json::json!(events),
            row["original"]["events"],
            "map insertion {case}"
        );
        assert_eq!(
            serde_json::json!(map.iter().collect::<Vec<_>>()),
            row["original"]["order"],
            "map order {case}"
        );
    }
}

#[cfg(feature = "private-fixtures")]
#[test]
fn native_pajama_selection_after_real_migrations_matches() {
    let report: serde_json::Value =
        serde_json::from_str(include_str!("../../../tests/fixtures/pajama-selector.json")).unwrap();
    struct Host<'a> {
        profile: Vec<String>,
        bodies: &'a [Vec<u8>],
        textures: &'a [Vec<u8>],
        events: Vec<serde_json::Value>,
        writes: Vec<Vec<String>>,
    }
    impl generator::Bank for Host<'_> {
        fn read(&mut self, entry: usize) -> Result<Option<Vec<u8>>, String> {
            self.events.push(serde_json::json!(["read", entry]));
            Ok(self.profile.get(entry - 1).map(|s| s.as_bytes().to_vec()))
        }
        fn set(&mut self, entry: usize, bytes: Vec<u8>) -> Result<(), String> {
            let value = String::from_utf8(bytes).unwrap();
            self.events.push(serde_json::json!(["set", entry, value]));
            self.profile[entry - 1] = value;
            Ok(())
        }
    }
    impl migration::Host for Host<'_> {
        fn generate(&mut self, generator: generator::Generator) -> Result<(), String> {
            generator::generate(generator, self, self.bodies, self.textures)
        }
        fn publish(&mut self) -> Result<(), String> {
            self.events.push(serde_json::json!(["publish"]));
            self.writes.push(self.profile.clone());
            Ok(())
        }
    }
    for (case, row) in report["cases"].as_array().unwrap().iter().enumerate() {
        let input = &row["input"];
        let names = |key: &str| -> Vec<Vec<u8>> {
            input[key]
                .as_array()
                .unwrap()
                .iter()
                .map(|s| s.as_str().unwrap().as_bytes().to_vec())
                .collect()
        };
        let bodies = names("bodies");
        let textures = names("textures");
        let mut host = Host {
            profile: serde_json::from_value(input["profile"].clone()).unwrap(),
            bodies: &bodies,
            textures: &textures,
            events: vec![serde_json::json!(["bind"])],
            writes: vec![],
        };
        migration::upgrade(&mut host).unwrap();
        let backup = host.profile.clone();
        host.events.push(serde_json::json!(["backup"]));
        let selected = pajama::select(
            input["person_age"].as_i64().unwrap() as i16,
            &mut host,
            &bodies,
            &textures,
        )
        .unwrap();
        assert_eq!(
            host.profile, backup,
            "selector changed the post-migration profile {case}"
        );
        host.events.extend([
            serde_json::json!(["restore"]),
            serde_json::json!(["clear_backup"]),
        ]);
        let original = &row["original"];
        assert_eq!(
            serde_json::json!(String::from_utf8(selected.body).unwrap()),
            original["body"],
            "body {case}"
        );
        assert_eq!(
            serde_json::json!(String::from_utf8(selected.texture).unwrap()),
            original["texture"],
            "texture {case}"
        );
        assert_eq!(
            serde_json::json!(host.profile),
            original["profile"],
            "profile {case}"
        );
        assert_eq!(
            serde_json::json!(host.writes),
            original["writes"],
            "writes {case}"
        );
        let events: Vec<_> = original["events"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|e| !matches!(e[0].as_str(), Some("body" | "body_count" | "textures")))
            .cloned()
            .collect();
        assert_eq!(host.events, events, "services {case}");
    }
}

#[cfg(feature = "private-fixtures")]
#[test]
fn native_profile_migrations_preserve_dispatch_and_writeback_order() {
    let report: serde_json::Value = serde_json::from_str(include_str!(
        "../../../tests/fixtures/clothing-profile-bind.json"
    ))
    .unwrap();
    struct Host {
        profile: Vec<String>,
        events: Vec<serde_json::Value>,
        writes: Vec<Vec<String>>,
    }
    impl generator::Bank for Host {
        fn read(&mut self, entry: usize) -> Result<Option<Vec<u8>>, String> {
            self.events.push(serde_json::json!(["read", entry]));
            Ok(self.profile.get(entry - 1).map(|s| s.as_bytes().to_vec()))
        }
        fn set(&mut self, entry: usize, value: Vec<u8>) -> Result<(), String> {
            let value = String::from_utf8(value).unwrap();
            self.events.push(serde_json::json!(["set", entry, value]));
            self.profile[entry - 1] = value;
            Ok(())
        }
    }
    impl migration::Host for Host {
        fn generate(&mut self, generator: generator::Generator) -> Result<(), String> {
            use generator::Generator;
            let (name, entries): (&str, &[usize]) = match generator {
                Generator::Adult => ("adult", &[31, 32, 33]),
                Generator::Child => ("child", &[32, 33]),
                Generator::HighFashion => ("high_fashion", &[35]),
                Generator::Winter => ("winter", &[34]),
            };
            self.events.push(serde_json::json!(["generate", name]));
            // Match the supplied generator service in this dispatch corpus.
            // Actual generator outputs have a separate unchanged-native corpus.
            for &entry in entries {
                self.profile[entry - 1] = format!("{name}-{entry}");
            }
            Ok(())
        }
        fn publish(&mut self) -> Result<(), String> {
            self.events.push(serde_json::json!(["write"]));
            self.writes.push(self.profile.clone());
            Ok(())
        }
    }
    for (i, row) in report["cases"].as_array().unwrap().iter().enumerate() {
        let mut host = Host {
            profile: serde_json::from_value(row["input"]["profile"].clone()).unwrap(),
            events: vec![],
            writes: vec![],
        };
        migration::upgrade(&mut host).unwrap();
        let native = &row["original"]["bound"];
        let events = native["events"].as_array().unwrap();
        assert_eq!(events.first().unwrap(), &serde_json::json!(["bind"]));
        assert_eq!(events.last().unwrap(), &serde_json::json!(["snapshot"]));
        assert_eq!(host.events, events[1..events.len() - 1], "dispatch {i}");
        assert_eq!(
            serde_json::json!(host.profile),
            native["profile"],
            "profile {i}"
        );
        assert_eq!(
            serde_json::json!(host.writes),
            native["writes"],
            "writebacks {i}"
        );
        assert_eq!(
            serde_json::json!(host.profile),
            native["snapshot"],
            "post-upgrade backup {i}"
        );
    }
}

#[cfg(feature = "private-fixtures")]
#[test]
fn native_outfit_generators_preserve_descriptors_and_profile_access_order() {
    let report: serde_json::Value = serde_json::from_str(include_str!(
        "../../../tests/fixtures/clothing-generators.json"
    ))
    .unwrap();
    struct ProfileBank {
        values: Vec<Vec<u8>>,
        events: Vec<serde_json::Value>,
    }
    impl generator::Bank for ProfileBank {
        fn read(&mut self, entry: usize) -> Result<Option<Vec<u8>>, String> {
            self.events.push(serde_json::json!(["read", entry]));
            Ok(self.values.get(entry - 1).cloned())
        }
        fn set(&mut self, entry: usize, value: Vec<u8>) -> Result<(), String> {
            self.events.push(serde_json::json!([
                "set",
                entry,
                String::from_utf8(value.clone()).unwrap()
            ]));
            self.values[entry - 1] = value;
            Ok(())
        }
    }
    for (case, row) in report["cases"].as_array().unwrap().iter().enumerate() {
        let input = &row["input"];
        let names = |key: &str| -> Vec<Vec<u8>> {
            input[key]
                .as_array()
                .unwrap()
                .iter()
                .map(|name| name.as_str().unwrap().as_bytes().to_vec())
                .collect()
        };
        let mut bank = ProfileBank {
            values: names("profile"),
            events: vec![],
        };
        generator::generate(
            serde_json::from_value(input["generator"].clone()).unwrap(),
            &mut bank,
            &names("bodies"),
            &names("textures"),
        )
        .unwrap();
        let actual: Vec<_> = bank
            .values
            .into_iter()
            .map(|value| String::from_utf8(value).unwrap())
            .collect();
        assert_eq!(
            serde_json::json!(actual),
            row["original"]["profile"],
            "profile {case}"
        );
        let expected: Vec<_> = row["original"]["events"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|event| matches!(event[0].as_str(), Some("read" | "set")))
            .cloned()
            .collect();
        assert_eq!(bank.events, expected, "profile access {case}");
    }
}

fn native_nodes(pattern: &Pattern) -> Vec<[i32; 5]> {
    pattern
        .nodes
        .iter()
        .map(|n| {
            [
                n.start as i32,
                n.length as i32,
                n.kind,
                n.next[0].map_or(-1, |i| i as i32),
                n.next[1].map_or(-1, |i| i as i32),
            ]
        })
        .collect()
}

#[cfg(feature = "private-fixtures")]
#[test]
fn native_clothing_templates_preserve_nodes_and_profile_read_order() {
    let report: serde_json::Value = serde_json::from_str(include_str!(
        "../../../tests/fixtures/clothing-template.json"
    ))
    .unwrap();
    struct Input {
        sex: i32,
        body: Vec<u8>,
        tone: Vec<u8>,
        reads: Vec<&'static str>,
    }
    impl Profile for Input {
        fn sex(&mut self) -> Result<i32, String> {
            self.reads.push("sex");
            Ok(self.sex)
        }
        fn body_type(&mut self) -> Result<Vec<u8>, String> {
            self.reads.push("body");
            // This native corpus supplies a zero-filled fixed descriptor buffer,
            // including its deliberately short synthetic body names.
            let mut buffer = self.body.clone();
            buffer.resize(64, 0);
            Ok(buffer[6..9].to_vec())
        }
        fn tone(&mut self) -> Result<Vec<u8>, String> {
            self.reads.push("tone");
            Ok(self.tone.clone())
        }
    }
    for (i, row) in report["cases"].as_array().unwrap().iter().enumerate() {
        let bytes = |key| serde_json::from_value::<Vec<u8>>(row["input"][key].clone()).unwrap();
        let mut input = Input {
            sex: row["input"]["sex"].as_i64().unwrap() as i32,
            body: bytes("body"),
            tone: bytes("tone"),
            reads: vec![],
        };
        let pattern = Pattern::expand(&bytes("template"), &mut input).unwrap();
        assert_eq!(
            serde_json::json!(native_nodes(&pattern)),
            row["original"]["nodes"],
            "nodes {i}"
        );
        assert_eq!(
            serde_json::json!(pattern.text),
            row["original"]["pool"],
            "pool {i}"
        );
        assert_eq!(
            serde_json::json!(input.reads),
            row["original"]["services"],
            "reads {i}"
        );
    }
}

#[cfg(feature = "private-fixtures")]
#[test]
fn native_pattern_builders_prefixes_and_ranked_candidates_match() {
    let report: serde_json::Value = serde_json::from_str(include_str!(
        "../../../tests/fixtures/clothing-pattern.json"
    ))
    .unwrap();
    let bytes = |v: &serde_json::Value| -> Vec<u8> { serde_json::from_value(v.clone()).unwrap() };
    for (case, row) in report["cases"].as_array().unwrap().iter().enumerate() {
        let mut pattern = Pattern::default();
        for op in row["input"]["operations"].as_array().unwrap() {
            if let Some(count) = op["skip"].as_i64() {
                pattern.skip(count as i32);
            } else if op["choice"].is_array() {
                pattern.either(&bytes(&op["choice"][0]), &bytes(&op["choice"][1]));
            } else {
                pattern.literal(&bytes(&op["literal"]));
            }
        }
        let nodes = native_nodes(&pattern);
        assert_eq!(
            serde_json::json!(nodes),
            row["original"]["nodes"],
            "builder {case}"
        );
        assert_eq!(pattern.text, bytes(&row["original"]["pool"]));
        let mut best = Best::default();
        let candidates: Vec<Vec<u8>> = row["input"]["candidates"]
            .as_array()
            .unwrap()
            .iter()
            .map(bytes)
            .collect();
        for (i, candidate) in candidates.iter().enumerate() {
            assert_eq!(
                serde_json::json!(pattern.matched_prefix(candidate)),
                row["original"]["scores"][i],
                "prefix {case}/{i}"
            );
            best.consider(&pattern, candidate);
            let native = &row["original"]["selected"][i];
            assert_eq!(best.name, bytes(&native["name"]), "selected {case}/{i}");
            assert_eq!(serde_json::json!(best.matched), native["matched"]);
            assert_eq!(
                serde_json::json!(best.fraction.to_bits()),
                native["fraction_bits"],
                "fraction {case}/{i}"
            );
        }
        let first = row["original"]["scores"]
            .as_array()
            .unwrap()
            .iter()
            .position(|v| v.as_u64().unwrap() != 0);
        assert_eq!(
            pattern.first(candidates.iter().map(Vec::as_slice)),
            first.map(|i| candidates[i].as_slice())
        );
    }
}
