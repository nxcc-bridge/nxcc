# @nxcc/cli

CLI for interacting with nXCC nodes

## Installation

```bash
npm install -g @nxcc/cli
```

## Commands

### Project Management

#### `nxcc init [directory]`

Create a new nXCC TypeScript project

```bash
nxcc init my-project
```

#### `nxcc bundle <manifest-template>`

Create a worker bundle from a manifest template

```bash
nxcc bundle manifest.json --out bundle.json --signer <private-key>
```

Options:

- `--out <path>`: Output path for the bundle
- `--signer <private-key>`: Private key to sign the bundle

### Worker Management

#### `nxcc worker deploy <manifest-path>`

Deploy a worker to an nXCC node

```bash
nxcc worker deploy manifest.json --rpc-url http://localhost:6922
```

Options:

- `--rpc-url <url>`: nXCC node HTTP RPC URL (default: http://localhost:6922)
- `--bundle`: Bundle the worker code into a data URL
- `--signer <private-key>`: Private key to sign the work order

#### `nxcc worker logs <worker-id>`

Stream logs from a worker

```bash
nxcc worker logs my-worker-id --rpc-url http://localhost:6922 --follow
```

Options:

- `--rpc-url <url>`: nXCC node HTTP RPC URL (default: http://localhost:6922)
- `-f, --follow`: Follow log output (stream new logs)
- `-t, --tail <lines>`: Number of lines to tail (default: 10)

### Node Management

#### `nxcc node get-report`

Get the node's environment report (attestation + operator signature)

```bash
nxcc node get-report --rpc-url http://localhost:6922
```

Options:

- `--rpc-url <url>`: nXCC node HTTP RPC URL (default: http://localhost:6922)
- `-o, --output <path>`: Output file to save the env report JSON

Example:

```bash
# Print env report to console
nxcc node get-report --rpc-url http://localhost:6922

# Save env report to file
nxcc node get-report --rpc-url http://localhost:6922 -o env-report.json
```

### Identity Management

#### `nxcc identity <command>`

Manage identities on various blockchains

## Examples

### Getting Started

1. Create a new project:

```bash
nxcc init my-nxcc-project
cd my-nxcc-project
```

2. Deploy a worker:

```bash
nxcc worker deploy manifest.json --rpc-url http://localhost:6922
```

3. View worker logs:

```bash
nxcc worker logs <worker-id> --follow --rpc-url http://localhost:6922
```

4. Get node environment report:

```bash
nxcc node get-report --rpc-url http://localhost:6922 -o node-report.json
```

## Configuration

The CLI uses the following default configuration:

- Default RPC URL: `http://localhost:6922`
- Default log tail lines: 10

You can override these defaults using command-line options.

## License

MIT
