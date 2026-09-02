# Nimino WSL launcher and secret boundary v1

The Windows adapter starts exactly `Ubuntu-24.04` with direct `wsl.exe` argv.
It accepts one validated Linux user, an absolute `/home` working directory, and
an absolute `/home` or `/usr` executable. `/mnt`, traversal, NUL, and shell
command strings are rejected.

The launched process must return this first stdout line:

```text
NIMINO_WSL_PID_V1<TAB>Ubuntu-24.04<TAB><user><TAB><linux-pid>
```

The adapter records that Linux PID together with the Windows host PID, exact
distro, and user. Dropping the owner kills and waits for the host process.

Secret bytes use one stdin frame: `NIMINO_SECRET_V1\0`, a four-byte big-endian
length, then the payload. The caller's buffer is zeroed after the write attempt.
The payload is never put in argv, the Windows environment, or a file. Inside
WSL, Linux Secret Service is the only durable identity backend.

Any predecessor plaintext identity is deleted without being parsed. Missing,
locked, corrupt, or unverifiable Secret Service state fails closed into
non-signing recovery; it never creates a file fallback.
