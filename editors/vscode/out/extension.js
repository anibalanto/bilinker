"use strict";
var __createBinding = (this && this.__createBinding) || (Object.create ? (function(o, m, k, k2) {
    if (k2 === undefined) k2 = k;
    var desc = Object.getOwnPropertyDescriptor(m, k);
    if (!desc || ("get" in desc ? !m.__esModule : desc.writable || desc.configurable)) {
      desc = { enumerable: true, get: function() { return m[k]; } };
    }
    Object.defineProperty(o, k2, desc);
}) : (function(o, m, k, k2) {
    if (k2 === undefined) k2 = k;
    o[k2] = m[k];
}));
var __setModuleDefault = (this && this.__setModuleDefault) || (Object.create ? (function(o, v) {
    Object.defineProperty(o, "default", { enumerable: true, value: v });
}) : function(o, v) {
    o["default"] = v;
});
var __importStar = (this && this.__importStar) || (function () {
    var ownKeys = function(o) {
        ownKeys = Object.getOwnPropertyNames || function (o) {
            var ar = [];
            for (var k in o) if (Object.prototype.hasOwnProperty.call(o, k)) ar[ar.length] = k;
            return ar;
        };
        return ownKeys(o);
    };
    return function (mod) {
        if (mod && mod.__esModule) return mod;
        var result = {};
        if (mod != null) for (var k = ownKeys(mod), i = 0; i < k.length; i++) if (k[i] !== "default") __createBinding(result, mod, k[i]);
        __setModuleDefault(result, mod);
        return result;
    };
})();
Object.defineProperty(exports, "__esModule", { value: true });
exports.activate = activate;
exports.deactivate = deactivate;
const vscode = __importStar(require("vscode"));
const path = __importStar(require("path"));
const os = __importStar(require("os"));
const fs = __importStar(require("fs"));
const child_process_1 = require("child_process");
const node_1 = require("vscode-languageclient/node");
let client;
function activate(context) {
    const serverBin = findBinary('bilinker-lsp');
    if (!serverBin) {
        vscode.window.showErrorMessage('bilinker-lsp not found. Run: cargo install --path crates/bilinker-lsp');
        return;
    }
    const serverOptions = {
        command: serverBin,
        transport: node_1.TransportKind.stdio,
    };
    const clientOptions = {
        // Activate for all file types — bilinks can reference any file
        documentSelector: [{ scheme: 'file', pattern: '**/*' }],
        synchronize: {
            fileEvents: vscode.workspace.createFileSystemWatcher('**/.bilink/**'),
        },
    };
    client = new node_1.LanguageClient('bilinker', 'Bilinker', serverOptions, clientOptions);
    client.start();
    // Command: open graph for current file
    context.subscriptions.push(vscode.commands.registerCommand('bilinker.openGraph', () => {
        const editor = vscode.window.activeTextEditor;
        if (!editor)
            return;
        openGraph(editor.document.uri.fsPath, false);
    }));
    // Command: open full system graph
    context.subscriptions.push(vscode.commands.registerCommand('bilinker.openSystemGraph', () => {
        const ws = vscode.workspace.workspaceFolders?.[0]?.uri.fsPath;
        if (!ws)
            return;
        openGraph(ws, true);
    }));
    // Handle code lens click to show bilinks in a panel
    context.subscriptions.push(vscode.commands.registerCommand('bilinker.showBilinks', async (uri, ids) => {
        const panel = vscode.window.createWebviewPanel('bilinkerBilinks', `Bilinks (${ids.length})`, vscode.ViewColumn.Beside, {});
        panel.webview.html = bilinksHtml(ids, uri);
    }));
}
function openGraph(filePath, recursive) {
    const bilinker = findBinary('bilinker');
    if (!bilinker) {
        vscode.window.showErrorMessage('bilinker not found in PATH');
        return;
    }
    const tmpFile = path.join(os.tmpdir(), 'bilinker-graph.html');
    const cwd = fs.statSync(filePath).isDirectory() ? filePath : path.dirname(filePath);
    const selector = '.';
    const args = [bilinker, 'graph', selector, '--format', 'html'];
    if (recursive)
        args.push('--recursive');
    vscode.window.withProgress({ location: vscode.ProgressLocation.Notification, title: 'Generating bilink graph…' }, () => new Promise((resolve, reject) => {
        const child = (0, child_process_1.spawn)(args[0], args.slice(1), { cwd });
        const chunks = [];
        child.stdout.on('data', (d) => chunks.push(d));
        child.stderr.on('data', (d) => console.error(d.toString()));
        child.on('close', (code) => {
            if (code !== 0) {
                reject(new Error(`bilinker exited ${code}`));
                return;
            }
            fs.writeFileSync(tmpFile, Buffer.concat(chunks));
            resolve();
        });
    })).then(() => {
        // Open in external browser
        vscode.env.openExternal(vscode.Uri.file(tmpFile));
    }, (err) => {
        vscode.window.showErrorMessage(`bilinker graph failed: ${err.message}`);
    });
}
function findBinary(name) {
    try {
        const result = (0, child_process_1.execSync)(`which ${name}`, { encoding: 'utf8' }).trim();
        return result || undefined;
    }
    catch {
        // Try ~/.cargo/bin as fallback
        const cargo = path.join(os.homedir(), '.cargo', 'bin', name);
        return fs.existsSync(cargo) ? cargo : undefined;
    }
}
function bilinksHtml(ids, uri) {
    const items = ids.map(id => `<li><code>${id}</code></li>`).join('');
    return `<!DOCTYPE html><html><body>
    <h3>Bilinks in <code>${path.basename(uri)}</code></h3>
    <ul>${items}</ul>
    </body></html>`;
}
function deactivate() {
    return client?.stop();
}
