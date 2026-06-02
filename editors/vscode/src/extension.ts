import * as vscode from 'vscode';
import * as path from 'path';
import * as os from 'os';
import * as fs from 'fs';
import { execSync, spawn } from 'child_process';
import {
    LanguageClient,
    LanguageClientOptions,
    ServerOptions,
    TransportKind,
} from 'vscode-languageclient/node';

let client: LanguageClient;

export function activate(context: vscode.ExtensionContext) {
    const serverBin = findBinary('bilinker-lsp');
    if (!serverBin) {
        vscode.window.showErrorMessage(
            'bilinker-lsp not found. Run: cargo install --path crates/bilinker-lsp'
        );
        return;
    }

    const serverOptions: ServerOptions = {
        command: serverBin,
        transport: TransportKind.stdio,
    };

    const clientOptions: LanguageClientOptions = {
        // Activate for all file types — bilinks can reference any file
        documentSelector: [{ scheme: 'file', pattern: '**/*' }],
        synchronize: {
            fileEvents: vscode.workspace.createFileSystemWatcher('**/.bilink/**'),
        },
    };

    client = new LanguageClient('bilinker', 'Bilinker', serverOptions, clientOptions);
    client.start();

    // Command: open graph for current file
    context.subscriptions.push(
        vscode.commands.registerCommand('bilinker.openGraph', () => {
            const editor = vscode.window.activeTextEditor;
            if (!editor) return;
            openGraph(editor.document.uri.fsPath, false);
        })
    );

    // Command: open full system graph
    context.subscriptions.push(
        vscode.commands.registerCommand('bilinker.openSystemGraph', () => {
            const ws = vscode.workspace.workspaceFolders?.[0]?.uri.fsPath;
            if (!ws) return;
            openGraph(ws, true);
        })
    );

    // Handle code lens click to show bilinks in a panel
    context.subscriptions.push(
        vscode.commands.registerCommand('bilinker.showBilinks', async (uri: string, ids: string[]) => {
            const panel = vscode.window.createWebviewPanel(
                'bilinkerBilinks',
                `Bilinks (${ids.length})`,
                vscode.ViewColumn.Beside,
                {}
            );
            panel.webview.html = bilinksHtml(ids, uri);
        })
    );
}

function openGraph(filePath: string, recursive: boolean) {
    const bilinker = findBinary('bilinker');
    if (!bilinker) {
        vscode.window.showErrorMessage('bilinker not found in PATH');
        return;
    }

    const tmpFile = path.join(os.tmpdir(), 'bilinker-graph.html');
    const cwd     = fs.statSync(filePath).isDirectory() ? filePath : path.dirname(filePath);
    const selector = '.';
    const args = [bilinker, 'graph', selector, '--format', 'html'];
    if (recursive) args.push('--recursive');

    vscode.window.withProgress(
        { location: vscode.ProgressLocation.Notification, title: 'Generating bilink graph…' },
        () => new Promise<void>((resolve, reject) => {
            const child = spawn(args[0], args.slice(1), { cwd });
            const chunks: Buffer[] = [];
            child.stdout.on('data', (d: Buffer) => chunks.push(d));
            child.stderr.on('data', (d: Buffer) => console.error(d.toString()));
            child.on('close', (code: number) => {
                if (code !== 0) { reject(new Error(`bilinker exited ${code}`)); return; }
                fs.writeFileSync(tmpFile, Buffer.concat(chunks));
                resolve();
            });
        })
    ).then(() => {
        // Open in external browser
        vscode.env.openExternal(vscode.Uri.file(tmpFile));
    }, (err: Error) => {
        vscode.window.showErrorMessage(`bilinker graph failed: ${err.message}`);
    });
}

function findBinary(name: string): string | undefined {
    try {
        const result = execSync(`which ${name}`, { encoding: 'utf8' }).trim();
        return result || undefined;
    } catch {
        // Try ~/.cargo/bin as fallback
        const cargo = path.join(os.homedir(), '.cargo', 'bin', name);
        return fs.existsSync(cargo) ? cargo : undefined;
    }
}

function bilinksHtml(ids: string[], uri: string): string {
    const items = ids.map(id =>
        `<li><code>${id}</code></li>`
    ).join('');
    return `<!DOCTYPE html><html><body>
    <h3>Bilinks in <code>${path.basename(uri)}</code></h3>
    <ul>${items}</ul>
    </body></html>`;
}

export function deactivate(): Thenable<void> | undefined {
    return client?.stop();
}
