# vim 配置示例

目前没有现成插件，建议根据实际使用情况自行配置。

以 vim + coc.nvim 为例，在 `coc-settings.json` 里加入如下配置（填入正确的程序路径）：

```jsonc
{
  "languageserver": {
    "rime-ls": {
      "command": "/usr/bin/rime_ls",
      "args": ["--connect"],
      "filetypes": ["text"],
    },
  },
}
```

Rime 目录在 `~/.config/rime-ls/config.yaml` 中配置。若服务端未运行，客户端会自动在后台启动。

> [!NOTE]
> 仅支持 Unix/Linux/macOS。Windows 下 `--connect` 不可用。

补充: 通过 `:call CocRequest('rime-ls', 'workspace/executeCommand', { 'command': 'rime-ls.toggle-rime' })` 可以手动控制开启和关闭
