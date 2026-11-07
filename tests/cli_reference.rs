use presswerk::cli::{CommandReference, reference};

fn subcommand<'a>(reference: &'a CommandReference, name: &str) -> &'a CommandReference {
    reference
        .subcommands
        .iter()
        .find(|command| command.name == name)
        .unwrap_or_else(|| panic!("missing subcommand {name}"))
}

#[test]
fn reference_lists_every_command_with_its_usage() {
    let reference = reference();

    assert_eq!(reference.name, "presswerk");
    assert_eq!(reference.usage, "presswerk [COMMAND]");
    let names: Vec<_> = reference
        .subcommands
        .iter()
        .map(|command| command.name.as_str())
        .collect();
    assert_eq!(
        names,
        ["presswerk plan", "presswerk prepare", "presswerk publish"]
    );
    assert_eq!(
        subcommand(&reference, "presswerk publish").usage,
        "presswerk publish [REVISION]"
    );
}

#[test]
fn reference_describes_arguments_and_options() {
    let reference = reference();

    let revision = &subcommand(&reference, "presswerk publish").arguments[0];
    assert_eq!(revision.id, "revision");
    assert_eq!(revision.value_names, ["REVISION"]);
    assert_eq!(revision.defaults, ["HEAD"]);
    assert!(!revision.required);

    let prepare = subcommand(&reference, "presswerk prepare");
    assert!(prepare.arguments.is_empty());
    let trust = &prepare.options[0];
    assert_eq!(trust.long.as_deref(), Some("trust-predecessor"));
    assert_eq!(trust.value_names, ["TAG=SHA"]);
    assert!(
        trust
            .help
            .as_deref()
            .unwrap()
            .starts_with("Trust one exact")
    );
}

#[test]
fn reference_omits_help_but_keeps_version() {
    let reference = reference();

    let longs: Vec<_> = reference
        .options
        .iter()
        .filter_map(|option| option.long.as_deref())
        .collect();
    assert_eq!(longs, ["version"]);
    assert!(
        subcommand(&reference, "presswerk plan").options.is_empty(),
        "plan has no options besides help"
    );
}
