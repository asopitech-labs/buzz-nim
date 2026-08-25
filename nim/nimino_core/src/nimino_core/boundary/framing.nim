import nimino_core/boundary/protocol

type BoundaryFrameError* = object of CatchableError
  code*: string

proc raiseFrameError(code, message: string) =
  var error = newException(BoundaryFrameError, message)
  error.code = code
  raise error

proc readExact(file: File; size: int): string =
  result = newString(size)
  var offset = 0
  while offset < size:
    let count = file.readBuffer(addr result[offset], size - offset)
    if count == 0:
      result.setLen(offset)
      return
    offset += count

proc readFrame*(file: File): tuple[available: bool, payload: string] =
  let header = readExact(file, 4)
  if header.len == 0:
    return (false, "")
  if header.len != 4:
    raiseFrameError("INVALID_REQUEST", "frame header was truncated")

  let length =
    (uint32(ord(header[0])) shl 24) or
    (uint32(ord(header[1])) shl 16) or
    (uint32(ord(header[2])) shl 8) or
    uint32(ord(header[3]))
  if length > uint32(BoundaryMaxFrameBytes):
    raiseFrameError("FRAME_TOO_LARGE", "frame exceeds the 1 MiB contract limit")

  let payload = readExact(file, int(length))
  if payload.len != int(length):
    raiseFrameError("INVALID_REQUEST", "frame payload was truncated")
  result = (true, payload)

proc writeFrame*(file: File; payload: string) =
  if payload.len > BoundaryMaxFrameBytes:
    raiseFrameError("FRAME_TOO_LARGE", "frame exceeds the 1 MiB contract limit")

  let length = uint32(payload.len)
  var header = newString(4)
  header[0] = char((length shr 24) and 0xff'u32)
  header[1] = char((length shr 16) and 0xff'u32)
  header[2] = char((length shr 8) and 0xff'u32)
  header[3] = char(length and 0xff'u32)
  file.write(header)
  file.write(payload)
  file.flushFile()
