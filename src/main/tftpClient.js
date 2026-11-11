const dgram = require('dgram');
const fs = require('fs');
const path = require('path');

const OP_RRQ = 1;
const OP_WRQ = 2;
const OP_DATA = 3;
const OP_ACK = 4;
const OP_ERROR = 5;
const BLOCK_SIZE = 512;
const TIMEOUT_MS = 3000;
const MAX_RETRIES = 5;

function buildRequest(opcode, filename) {
  const header = Buffer.alloc(2);
  header.writeUInt16BE(opcode, 0);
  const mode = Buffer.from('octet\0', 'ascii');
  return Buffer.concat([header, Buffer.from(filename + '\0', 'ascii'), mode]);
}

function buildAck(block) {
  const buf = Buffer.alloc(4);
  buf.writeUInt16BE(OP_ACK, 0);
  buf.writeUInt16BE(block & 0xffff, 2);
  return buf;
}

function buildData(block, chunk) {
  const header = Buffer.alloc(4);
  header.writeUInt16BE(OP_DATA, 0);
  header.writeUInt16BE(block & 0xffff, 2);
  return Buffer.concat([header, chunk]);
}

function parseError(buf) {
  const code = buf.readUInt16BE(2);
  const msg = buf.slice(4, buf.length - 1).toString('ascii');
  return `TFTP error ${code}: ${msg}`;
}

// Downloads a file via TFTP RRQ into localDir, returns the saved local path.
function download(host, port, remoteFileName, localDir) {
  return new Promise((resolve, reject) => {
    const socket = dgram.createSocket('udp4');
    const localPath = path.join(localDir, path.basename(remoteFileName));
    const fd = fs.openSync(localPath, 'w');
    let remotePort = null;
    let expectedBlock = 1;
    let bytesWritten = 0;
    let retries = 0;
    let timer = null;
    let finished = false;

    const cleanup = (err) => {
      if (finished) return;
      finished = true;
      clearTimeout(timer);
      socket.close();
      fs.closeSync(fd);
      if (err) {
        fs.unlink(localPath, () => {});
        reject(err);
      } else {
        resolve({ localPath, bytes: bytesWritten });
      }
    };

    const armTimeout = (onTimeout) => {
      clearTimeout(timer);
      timer = setTimeout(onTimeout, TIMEOUT_MS);
    };

    const sendAck = (block) => {
      socket.send(buildAck(block), remotePort, host);
    };

    socket.once('error', (err) => cleanup(err));

    socket.on('message', (msg, rinfo) => {
      if (remotePort === null) remotePort = rinfo.port;
      const opcode = msg.readUInt16BE(0);

      if (opcode === OP_ERROR) return cleanup(new Error(parseError(msg)));
      if (opcode !== OP_DATA) return; // ignore unexpected packets

      const block = msg.readUInt16BE(2);
      if (block !== (expectedBlock & 0xffff)) {
        // Duplicate or out-of-order block: re-ack last good block, ignore data.
        sendAck((expectedBlock - 1) & 0xffff);
        return;
      }

      const chunk = msg.slice(4);
      fs.writeSync(fd, chunk, 0, chunk.length, bytesWritten);
      bytesWritten += chunk.length;
      retries = 0;
      sendAck(block);

      if (chunk.length < BLOCK_SIZE) {
        cleanup(null); // last block
        return;
      }
      expectedBlock += 1;
      armTimeout(() => {
        if (retries++ >= MAX_RETRIES) return cleanup(new Error('TFTP transfer timed out'));
        sendAck((expectedBlock - 1) & 0xffff); // ask source to resend by re-acking previous
      });
    });

    const request = buildRequest(OP_RRQ, remoteFileName);
    socket.send(request, port, host, (err) => {
      if (err) return cleanup(err);
      armTimeout(() => {
        if (retries++ >= MAX_RETRIES) return cleanup(new Error('TFTP server did not respond'));
        socket.send(request, port, host);
      });
    });
  });
}

// Uploads a local file via TFTP WRQ, writing it to the server under remoteFileName.
function upload(host, port, localPath, remoteFileName) {
  return new Promise((resolve, reject) => {
    const socket = dgram.createSocket('udp4');
    const fd = fs.openSync(localPath, 'r');
    const totalSize = fs.statSync(localPath).size;
    let remotePort = null;
    let offset = 0;
    let retries = 0;
    let timer = null;
    let finished = false;
    let lastSent = null;
    let lastSentBlock = 0; // 16-bit block number of the packet currently awaiting ack
    let awaitingInitialAck = true;

    const cleanup = (err) => {
      if (finished) return;
      finished = true;
      clearTimeout(timer);
      socket.close();
      fs.closeSync(fd);
      if (err) reject(err);
      else resolve({ bytes: totalSize });
    };

    const armTimeout = () => {
      clearTimeout(timer);
      timer = setTimeout(() => {
        if (retries++ >= MAX_RETRIES) return cleanup(new Error('TFTP transfer timed out'));
        if (awaitingInitialAck) socket.send(buildRequest(OP_WRQ, remoteFileName), port, host);
        else socket.send(lastSent, remotePort, host);
      }, TIMEOUT_MS);
    };

    const sendDataBlock = () => {
      const toRead = Math.min(BLOCK_SIZE, totalSize - offset);
      const buf = Buffer.alloc(toRead);
      if (toRead > 0) fs.readSync(fd, buf, 0, toRead, offset);
      const blockNum = (lastSentBlock + 1) & 0xffff;
      const packet = buildData(blockNum, buf);
      lastSent = packet;
      lastSentBlock = blockNum;
      offset += toRead;
      socket.send(packet, remotePort, host);
      armTimeout();
    };

    socket.once('error', (err) => cleanup(err));

    socket.on('message', (msg, rinfo) => {
      if (remotePort === null) remotePort = rinfo.port;
      const opcode = msg.readUInt16BE(0);
      if (opcode === OP_ERROR) return cleanup(new Error(parseError(msg)));
      if (opcode !== OP_ACK) return;
      const ackedBlock = msg.readUInt16BE(2);

      if (awaitingInitialAck) {
        if (ackedBlock !== 0) return;
        awaitingInitialAck = false;
        retries = 0;
        lastSentBlock = 0;
        sendDataBlock();
        return;
      }

      if (ackedBlock !== lastSentBlock) return; // stale/duplicate ack, ignore
      retries = 0;

      const lastPacketWasShort = lastSent.length - 4 < BLOCK_SIZE;
      if (lastPacketWasShort) {
        cleanup(null); // that short (possibly empty) block was the final one
        return;
      }
      sendDataBlock();
    });

    const request = buildRequest(OP_WRQ, remoteFileName);
    socket.send(request, port, host, (err) => {
      if (err) return cleanup(err);
      armTimeout();
    });
  });
}

module.exports = { download, upload };
