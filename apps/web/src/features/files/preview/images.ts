// Container guards run before any browser image decode. They do not validate
// compressed pixel data or promise a codec sandbox / peak-memory bound.
type Size = {
    width: number;
    height: number;
};
const malformed = () => new Error("图片容器损坏或结构不支持，请下载原文件。");
function requireValid(condition: unknown): asserts condition { if (!condition)
    throw malformed(); }
function pixels(width: number, height: number): Size {
    requireValid(Number.isInteger(width) && Number.isInteger(height) && width > 0 && height > 0);
    if (width > Math.floor(40000000 / height))
        throw new Error("图片预览限 40,000,000 像素，请下载原文件。");
    return { width, height };
}
const byte = (bytes: Uint8Array, at: number): number => { const value = bytes[at]; requireValid(value !== undefined); return value; };
const ascii = (bytes: Uint8Array, start: number, length: number) => String.fromCharCode(...bytes.subarray(start, start + length));
const view = (bytes: Uint8Array) => new DataView(bytes.buffer, bytes.byteOffset, bytes.byteLength);
function png(bytes: Uint8Array): Size {
    requireValid(bytes.length >= 33 && [137, 80, 78, 71, 13, 10, 26, 10].every((v, i) => byte(bytes, i) === v));
    const data = view(bytes);
    let at = 8, size: Size | undefined, hasData = false, endedData = false;
    while (at < bytes.length) {
        requireValid(bytes.length - at >= 12);
        const length = data.getUint32(at), type = ascii(bytes, at + 4, 4);
        requireValid(length <= bytes.length - at - 12 && /^[A-Za-z]{4}$/.test(type));
        const end = at + length + 12;
        if (type === "IHDR") {
            requireValid(at === 8 && !size && length === 13);
            size = pixels(data.getUint32(at + 8), data.getUint32(at + 12));
            const depth = byte(bytes, at + 16), color = byte(bytes, at + 17);
            const depths: Record<number, number[]> = { 0: [1, 2, 4, 8, 16], 2: [8, 16], 3: [1, 2, 4, 8], 4: [8, 16], 6: [8, 16] };
            requireValid(depths[color]?.includes(depth) && byte(bytes, at + 18) === 0 && byte(bytes, at + 19) === 0 && byte(bytes, at + 20) <= 1);
        }
        else {
            requireValid(size);
            requireValid(!["acTL", "fcTL", "fdAT"].includes(type));
            if (type === "IEND") {
                requireValid(length === 0 && hasData && end === bytes.length);
                return size;
            }
            if (type === "IDAT") {
                requireValid(!endedData);
                hasData = true;
            }
            else if (hasData)
                endedData = true;
            // Unknown critical chunks cannot safely be interpreted as this static PNG.
            requireValid(type === "IDAT" || type === "PLTE" || type[0] === type[0]!.toLowerCase());
        }
        at = end;
    }
    throw malformed();
}
function jpeg(bytes: Uint8Array): Size {
    requireValid(byte(bytes, 0) === 0xff && byte(bytes, 1) === 0xd8);
    const data = view(bytes);
    let at = 2, size: Size | undefined, inScan = false, scanned = false;
    let components: number[] = [];
    while (at < bytes.length) {
        if (inScan) {
            // Entropy data can include escaped FF and restart markers. Other markers
            // (including malicious second SOF/DNL) return to the segment parser.
            while (at < bytes.length) {
                if (byte(bytes, at++) !== 0xff)
                    continue;
                const markerStart = at - 1;
                while (byte(bytes, at) === 0xff)
                    at++;
                requireValid(at < bytes.length);
                if (byte(bytes, at) === 0 || (byte(bytes, at) >= 0xd0 && byte(bytes, at) <= 0xd7)) {
                    at++;
                    continue;
                }
                at = markerStart;
                inScan = false;
                break;
            }
            requireValid(!inScan);
        }
        requireValid(byte(bytes, at++) === 0xff);
        while (byte(bytes, at) === 0xff)
            at++;
        requireValid(at < bytes.length);
        const marker = byte(bytes, at++);
        if (marker === 0xd9) {
            requireValid(size && scanned && at === bytes.length);
            return size;
        }
        requireValid(marker !== 0xdc); // DNL would defer/redefine dimensions.
        requireValid(bytes.length - at >= 2);
        const length = data.getUint16(at);
        requireValid(length >= 2 && length <= bytes.length - at);
        const end = at + length;
        const frame = marker >= 0xc0 && marker <= 0xcf && ![0xc4, 0xc8, 0xcc].includes(marker);
        if (frame) {
            requireValid((marker === 0xc0 || marker === 0xc2) && !size && !scanned && length >= 8 && byte(bytes, at + 2) === 8);
            const count = byte(bytes, at + 7);
            requireValid([1, 3, 4].includes(count) && length === 8 + count * 3);
            size = pixels(data.getUint16(at + 5), data.getUint16(at + 3));
            components = Array.from({ length: count }, (_, i) => byte(bytes, at + 8 + i * 3));
            requireValid(new Set(components).size === count);
            for (let i = 0; i < count; i++) {
                const sampling = byte(bytes, at + 9 + i * 3);
                requireValid((sampling >> 4) >= 1 && (sampling >> 4) <= 4 && (sampling & 15) >= 1 && (sampling & 15) <= 4 && byte(bytes, at + 10 + i * 3) <= 3);
            }
        }
        else if (marker === 0xda) {
            requireValid(size && length >= 6);
            const count = byte(bytes, at + 2);
            requireValid(count > 0 && count <= components.length && length === 6 + count * 2);
            const selected = Array.from({ length: count }, (_, i) => byte(bytes, at + 3 + i * 2));
            requireValid(new Set(selected).size === count && selected.every(c => components.includes(c)));
            inScan = true;
            scanned = true;
        }
        else {
            requireValid([0xc4, 0xdb, 0xdd, 0xfe].includes(marker) || (marker >= 0xe0 && marker <= 0xef));
            if (marker === 0xdd)
                requireValid(length === 4);
        }
        at = end;
    }
    throw malformed();
}
function webp(bytes: Uint8Array): Size {
    requireValid(bytes.length >= 12 && ascii(bytes, 0, 4) === "RIFF" && ascii(bytes, 8, 4) === "WEBP");
    const data = view(bytes);
    requireValid(data.getUint32(4, true) === bytes.length - 8);
    let at = 12, canvas: Size | undefined, bitstream: Size | undefined;
    const u24 = (p: number) => byte(bytes, p) + byte(bytes, p + 1) * 256 + byte(bytes, p + 2) * 65536;
    while (at < bytes.length) {
        requireValid(bytes.length - at >= 8);
        const type = ascii(bytes, at, 4), length = data.getUint32(at + 4, true), start = at + 8;
        requireValid(length <= bytes.length - start && length + (length % 2) <= bytes.length - start);
        if (length % 2)
            requireValid(byte(bytes, start + length) === 0);
        requireValid(type !== "ANIM" && type !== "ANMF");
        if (type === "VP8X") {
            requireValid(at === 12 && !canvas && length === 10 && (byte(bytes, start) & 0xc3) === 0 && byte(bytes, start + 1) === 0 && byte(bytes, start + 2) === 0 && byte(bytes, start + 3) === 0);
            canvas = pixels(u24(start + 4) + 1, u24(start + 7) + 1);
        }
        else if (type === "VP8 ") {
            requireValid(!bitstream && length >= 10 && (byte(bytes, start) & 1) === 0 && byte(bytes, start + 3) === 0x9d && byte(bytes, start + 4) === 1 && byte(bytes, start + 5) === 0x2a);
            bitstream = pixels(data.getUint16(start + 6, true) & 0x3fff, data.getUint16(start + 8, true) & 0x3fff);
        }
        else if (type === "VP8L") {
            requireValid(!bitstream && length >= 5 && byte(bytes, start) === 0x2f);
            const bits = data.getUint32(start + 1, true);
            requireValid((bits >>> 29) === 0);
            bitstream = pixels((bits & 0x3fff) + 1, ((bits >>> 14) & 0x3fff) + 1);
        }
        else {
            requireValid(canvas && ["ICCP", "ALPH", "EXIF", "XMP "].includes(type));
        }
        at = start + length + (length % 2);
    }
    requireValid(bitstream && (!canvas || (canvas.width === bitstream.width && canvas.height === bitstream.height)));
    return bitstream;
}
export function inspectImage(bytes: Uint8Array, mime: string): Size {
    switch (mime) {
        case "image/png": return png(bytes);
        case "image/jpeg": return jpeg(bytes);
        case "image/webp": return webp(bytes);
        default: throw malformed();
    }
}
