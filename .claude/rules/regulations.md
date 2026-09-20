# Regulations

- Read .claude/settings.json and .claude/setting.local.json to check the permitted commands and avoid asking permissions by using the commands in the file.
- When a new grammer is implemented, an example code to check if it works must be generated in example folder. And if error pattern is implemented, an error example is also neede, whose name has "_error" at the last.
- When Python implementation is updated, also update the git SHA to track and syncronize the versions between the Rust-implementation and Python-implementation.
- If running the same script(s) many times, make them .ps1 file to ease command permission.
- When VS code extension is updated, the compilation and the generation of VSIX file are required. To generate VSIX filee, run make-vsix.ps1.
- 番号の振り方は1. 大きなタスクの纏まりごとにフェーズを作り、その下位にタスクを振り分ける。この時、異存関係を意識して、前提となるタスクは手前のフェーズにする。2. フェーズとタスクごとに番号を振り分ける。タスクは1-1, 1-2, ..., 2-1, 2-2, ...とする。ファイル内で被らなければ、他のファイルと番号が被っても良い
  - ただし、フェーズに分ける必要がない時にはフェーズに無理に切り分けず、#1, #2, ...と採番する。
  - タスク番号に英字を使用することは禁止します。
- 特に指定がない限り、タスク完了ごとに順次現在のブランチでコミットを作成する。ただし、pushは行わないこと。
