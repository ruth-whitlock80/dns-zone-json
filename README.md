# dnsconv

A converter between BIND-style DNS zone files and a JSON representation of
the same records.

Zone files are the format almost every authoritative DNS server reads and
writes, but they're a pain to generate or consume from a script: owner
names can be blank and inherit from the line above, `$ORIGIN` changes how
relative names get qualified partway through a file, TTLs can come from a
per-record field or a file-wide `$TTL` directive, and TXT records have their
own quoting rules. JSON is easier to build from code, diff, or feed into
other tooling. This tool goes back and forth between the two, and tries to
get the zone-file quirks right rather than just handling the easy 80%.

## Usage

Convert a zone file to JSON:

```
$ cat zone.txt
$ORIGIN example.com.
$TTL 3600
@       IN  NS      ns1.example.com.
ns1     IN  A       203.0.113.10
www     IN  A       203.0.113.20
        IN  A       203.0.113.21
mail    IN  MX  10  mailhost.example.com.
info    IN  TXT     "v=spf1 -all"

$ dnsconv zone.txt
[
  {
    "name": "example.com.",
    "ttl": 3600,
    "type": "NS",
    "target": "ns1.example.com."
  },
  {
    "name": "ns1.example.com.",
    "ttl": 3600,
    "type": "A",
    "address": "203.0.113.10"
  },
  {
    "name": "www.example.com.",
    "ttl": 3600,
    "type": "A",
    "address": "203.0.113.20"
  },
  {
    "name": "www.example.com.",
    "ttl": 3600,
    "type": "A",
    "address": "203.0.113.21"
  },
  {
    "name": "mail.example.com.",
    "ttl": 3600,
    "type": "MX",
    "preference": 10,
    "exchange": "mailhost.example.com."
  },
  {
    "name": "info.example.com.",
    "ttl": 3600,
    "type": "TXT",
    "text": [
      "v=spf1 -all"
    ]
  }
]
```

Note the blank owner name on the second `A` record for `www` — it inherits
the owner from the line above, a common shorthand in hand-written zones.

Convert JSON back to a zone file (direction is chosen by the `.json`
extension):

```
$ dnsconv records.json
example.com.    3600    IN  NS  ns1.example.com.
ns1.example.com.    3600    IN  A   203.0.113.10
www.example.com.    3600    IN  A   203.0.113.20
www.example.com.    3600    IN  A   203.0.113.21
mail.example.com.    3600    IN  MX  10 mailhost.example.com.
info.example.com.    3600    IN  TXT "v=spf1 -all"
```

If a zone file uses relative names before its first `$ORIGIN` line, pass
`--origin`:

```
$ dnsconv zone.txt --origin example.com.
```

## Supported record types

`A`, `AAAA`, `CNAME`, `MX`, `NS`, `TXT`. Anything else is a parse error
rather than being silently dropped.

## Known limitations

- No support for parenthesized multi-line records (`( ... )` spanning
  several lines).
- No `$INCLUDE`.
- No `SOA`, `PTR`, `SRV`, or `CAA` yet.
- An explicit per-record TTL does not become the new default for records
  after it the way some implementations do; only `$TTL` sets the default.

## Building

Standard `cargo build`. No external dependencies.

## Testing

`cargo test` runs a table-driven suite in `src/zone.rs` covering the
awkward parsing cases: owner name inheritance, `$ORIGIN` changes mid-file,
`@`, escaped quotes and semicolons inside TXT strings, case-insensitive
keywords, and missing-TTL errors.
