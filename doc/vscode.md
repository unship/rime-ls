# VSCode 配置示例

用官方的 LSP 插件的[例子](https://github.com/microsoft/vscode-extension-samples/tree/main/lsp-sample)
稍加修改，将启动命令设为 `rime_ls --connect`：

```typescript
export function activate(context: vscode.ExtensionContext) {
  const executable = {
    command: "/path/to/rime_ls",
    args: ["--connect"],
  };
  const serverOptions: ServerOptions = {
    run: executable,
    debug: executable,
  };

  const clientOptions: LanguageClientOptions = {
    documentSelector: [{ scheme: "file", language: "plaintext" }],
  };

  client = new LanguageClient(
    "Rime_LSP_Example",
    "Rime LSP Example",
    serverOptions,
    clientOptions,
  );
  client.start();
}
```

Rime 目录在 `~/.config/rime-ls/config.yaml` 中配置。若服务端未运行，客户端会自动在后台启动。

> [!NOTE]
> 仅支持 Unix/Linux/macOS。Windows 下 `--connect` 不可用。
