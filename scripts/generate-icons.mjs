/** 仅生成三个桌面占位图标，使用 Node 内置 PNG 压缩与容器编码，不运行项目或验证器。 */
import { mkdirSync, writeFileSync } from 'node:fs'
import { fileURLToPath } from 'node:url'
import { deflateSync } from 'node:zlib'

const output = new URL('../src-tauri/icons/', import.meta.url)
mkdirSync(output, { recursive: true })

function crc32(buffer) {
  let crc = 0xffffffff
  for (const byte of buffer) {
    crc ^= byte
    for (let bit = 0; bit < 8; bit += 1) crc = (crc >>> 1) ^ ((crc & 1) ? 0xedb88320 : 0)
  }
  return (crc ^ 0xffffffff) >>> 0
}

function chunk(type, data) {
  const name = Buffer.from(type, 'ascii')
  const length = Buffer.alloc(4)
  length.writeUInt32BE(data.length)
  const checksum = Buffer.alloc(4)
  checksum.writeUInt32BE(crc32(Buffer.concat([name, data])))
  return Buffer.concat([length, name, data, checksum])
}

function png(size) {
  const rows = Buffer.alloc((size * 4 + 1) * size)
  for (let y = 0; y < size; y += 1) {
    for (let x = 0; x < size; x += 1) {
      const nx = x / size
      const ny = y / size
      const offset = y * (size * 4 + 1) + 1 + x * 4
      const within = nx >= 0.08 && nx <= 0.92 && ny >= 0.08 && ny <= 0.92
      const vertical = ((nx >= 0.25 && nx <= 0.32) || (nx >= 0.68 && nx <= 0.75)) && ny >= 0.27 && ny <= 0.73
      const diagonal = nx >= 0.28 && nx <= 0.72 && Math.abs(ny - (0.55 - Math.abs(nx - 0.5))) < 0.035
      const color = vertical || diagonal ? [247, 248, 245] : [38, 107, 88]
      rows[offset] = color[0]
      rows[offset + 1] = color[1]
      rows[offset + 2] = color[2]
      rows[offset + 3] = within ? 255 : 0
    }
  }
  const header = Buffer.alloc(13)
  header.writeUInt32BE(size, 0)
  header.writeUInt32BE(size, 4)
  header[8] = 8
  header[9] = 6
  return Buffer.concat([
    Buffer.from([137, 80, 78, 71, 13, 10, 26, 10]),
    chunk('IHDR', header), chunk('IDAT', deflateSync(rows)), chunk('IEND', Buffer.alloc(0)),
  ])
}

const large = png(512)
const windows = png(256)
const icoHeader = Buffer.alloc(22)
icoHeader.writeUInt16LE(1, 2)
icoHeader.writeUInt16LE(1, 4)
icoHeader.writeUInt16LE(1, 10)
icoHeader.writeUInt16LE(32, 12)
icoHeader.writeUInt32LE(windows.length, 14)
icoHeader.writeUInt32LE(22, 18)
const icnsHeader = Buffer.alloc(16)
icnsHeader.write('icns', 0, 'ascii')
icnsHeader.writeUInt32BE(16 + large.length, 4)
icnsHeader.write('ic09', 8, 'ascii')
icnsHeader.writeUInt32BE(8 + large.length, 12)

const resources = [
  ['icon.png', large],
  ['icon.ico', Buffer.concat([icoHeader, windows])],
  ['icon.icns', Buffer.concat([icnsHeader, large])],
]
for (const [name, bytes] of resources) {
  const target = new URL(name, output)
  writeFileSync(target, bytes)
  console.log(`已生成 ${fileURLToPath(target)}`)
}
