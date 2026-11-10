// Real protocol fixture used through pipes and an actual HTTP socket.
const fs = require('node:fs');
const mode = process.argv[2];
const log = process.argv[3];
function receive(message, reply, disconnect) {
  if (message.id === undefined) return reply(null);
  let result;
  if (message.method === 'initialize') result = { protocolVersion: message.params.protocolVersion, capabilities: { tools: {} }, serverInfo: { name: 'fixture', version: '1' } };
  else if (message.method === 'tools/list') result = { tools: ['echo', 'fail', 'disconnect', 'hang'].map(name => ({ name, inputSchema: { type: 'object', properties: { text: { type: 'string' } }, additionalProperties: false }, ...(name === 'echo' ? { outputSchema: { type: 'object', required: ['text'], properties: { text: { type: 'string' } } } } : {}) })) };
  else if (message.method === 'tools/call') {
    fs.appendFileSync(log, `${message.params.name}\n`);
    if (message.params.name === 'disconnect') return disconnect();
    if (message.params.name === 'hang') return;
    result = { content: [{ type: 'text', text: message.params.arguments.text || 'fixture' }], isError: message.params.name === 'fail' };
    if (message.params.name === 'echo') result.structuredContent = { text: message.params.arguments.text === 'invalid-output' ? 17 : message.params.arguments.text || 'fixture' };
  } else return reply({ jsonrpc: '2.0', id: message.id, error: { code: -32601, message: 'unknown' } });
  reply({ jsonrpc: '2.0', id: message.id, result });
}
if (mode === 'stdio') {
  require('node:readline').createInterface({ input: process.stdin }).on('line', line => receive(JSON.parse(line), result => { if (result) process.stdout.write(JSON.stringify(result) + '\n'); }, () => process.exit(0)));
} else {
  require('node:http').createServer((request, response) => {
    if (request.method === 'GET') { response.writeHead(405).end(); return; }
    if (request.method === 'DELETE') { response.writeHead(200).end(); return; }
    let body = ''; request.on('data', chunk => body += chunk);
    request.on('end', () => { const message = JSON.parse(body); receive(message, result => {
      if (!result) response.writeHead(202).end();
      else if (message.method === 'tools/call' && message.params.name === 'echo') { response.writeHead(200, { 'Content-Type': 'text/event-stream' }); response.end(`event: message\ndata: ${JSON.stringify(result)}\n\n`); }
      else { response.writeHead(200, { 'Content-Type': 'application/json', 'Mcp-Session-Id': 'fixture-session' }); response.end(JSON.stringify(result)); }
    }, () => { response.destroy(); }); });
  }).listen(0, '127.0.0.1', function () { fs.writeFileSync(process.argv[4], String(this.address().port)); });
}
