use super::Cli;
use clap::CommandFactory;

fn collect_leaf_commands(command: &clap::Command, prefix: &str, out: &mut Vec<String>) {
    let children: Vec<_> = command
        .get_subcommands()
        .filter(|child| child.get_name() != "help")
        .collect();
    if children.is_empty() {
        out.push(prefix.to_owned());
    } else {
        for child in children {
            let path = if prefix.is_empty() {
                child.get_name().to_owned()
            } else {
                format!("{prefix}.{}", child.get_name())
            };
            collect_leaf_commands(child, &path, out);
        }
    }
}

#[test]
fn clap_leaf_grammar_matches_nimino_v1_contract() {
    let contract: serde_json::Value = serde_json::from_str(include_str!(
        "../../../contracts/nimino-cli/v1/commands.json"
    ))
    .expect("valid command contract");
    let mut expected: Vec<_> = contract["paths"]
        .as_array()
        .expect("paths")
        .iter()
        .map(|path| path.as_str().expect("string path").to_owned())
        .collect();
    let mut actual = Vec::new();
    collect_leaf_commands(&Cli::command(), "", &mut actual);
    expected.sort();
    actual.sort();
    assert_eq!(actual, expected);
}
