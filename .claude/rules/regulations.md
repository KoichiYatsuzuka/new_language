# Regulations

- Read .claude/settings.json and .claude/setting.local.json to check the permitted commands and avoid asking permissions by using the commands in the file.
- When a new grammer is implemented, an example code to check if it works must be generated in example folder. And if error pattern is implemented, an error example is also neede, whose name has "_error" at the last.
- When Python implementation is updated, also update the git SHA to track and syncronize the versions between the Rust-implementation and Python-implementation.
- If running the same script(s) many times, make them .ps1 file to ease command permission.
- When VS code extension is updated, the compilation and the generation of VSIX file are required. To generate VSIX filee, run make-vsix.ps1.
