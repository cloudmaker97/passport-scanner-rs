# Browser and Other Programs

Both features are off by default. They only work on this computer; other
computers on the network cannot connect.

## Weboberfläche (web interface)

Lets you use the scanner from a web browser on the same PC.

1. Open **Einstellungen** and switch on **Weboberfläche**.
2. Open <http://127.0.0.1:8733> in your browser. **Adresse kopieren**
   (*copy address*) puts the address on the clipboard.

The browser shows the same window as the program, with the same documents and
buttons. The program window must stay open. If you close it, the browser page
says it cannot reach the scanner service and connects again by itself once the
program is back.

A few differences:

- The switches for **Weboberfläche** and **MCP** are only in the program window.
- **PDF** downloads the file like any other download from the browser.
- The language and appearance you choose in the browser are remembered by that
  browser, separately from the program window.

The page needs a current browser (Chrome, Edge, Firefox or Safari) with
JavaScript and WebAssembly switched on.

## MCP (for AI assistants)

MCP lets an AI assistant on your PC, such as Claude Desktop, read the MRZ data
of scanned documents. It can only read. It cannot scan, change settings or
delete anything.

1. Open **Einstellungen** and switch on **MCP**.
2. In your assistant, add a server that runs `passport-scanner-mcp.exe` from
   the program folder. Example for Claude Desktop:

```json
{
  "mcpServers": {
    "passport-scanner": {
      "command": "C:\\Programs\\Passport Scanner\\passport-scanner-mcp.exe",
      "args": []
    }
  }
}
```

The assistant can then list the documents and read their MRZ data. Remember
that this is personal data.
