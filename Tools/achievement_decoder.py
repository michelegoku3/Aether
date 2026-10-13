#!/usr/bin/env python3
# ============================================================================
# achievement_decoder.py — decoder/encoder for Steam achievement cache files
# + handling of the JSON snapshot backups created by AetherDLL.
#
# COMMANDS
#
#   1) Decode (default) — pass it one or more files:
#
#        python achievement_decoder.py UserGameStatsSchema_1817070.bin \
#                                     UserGameStats_1916127674_1817070.bin
#
#      Decodes schema/stats .bin files (as well as AetherDLL backup .json
#      snapshots) and prints the unlock timeline with names. The schema is
#      only needed to translate bits into names.
#
#   2) snapshot — converts a Steam .bin into the same JSON format used by the
#      AetherDLL backup (useful to "seed" the backup with achievements
#      already unlocked BEFORE installing the new build):
#
#        python achievement_decoder.py snapshot UserGameStats_1916127674_1817070.bin \
#                --schema UserGameStatsSchema_1817070.bin \
#                -o UserGameStats_1916127674_1817070.json
#
#   3) rebuild — rebuilds a valid .bin for Steam from a backup
#      snapshot JSON (to put back into <steam>\appcache\stats\ with Steam
#      closed so achievements reappear):
#
#        python achievement_decoder.py rebuild UserGameStats_1916127674_1817070.json \
#                --schema UserGameStatsSchema_1817070.bin \
#                -o UserGameStats_1916127674_1817070.bin
#
# INTERNAL FORMAT (Valve binary VDF)
#   0x00 = dictionary open (followed by key\0)
#   0x01 = string  (key\0 value\0)
#   0x02 = int32    (key\0 + 4 little-endian bytes)
#   0x03 = float32  (key\0 + 4 bytes)
#   0x08 = dictionary close
#
# Layout of UserGameStats_<account>_<appid>.bin:
#   cache { crc=0, PendingChanges>0 if there are changes not confirmed by the server,
#           <bucket> { data=bitfield32, state, AchievementTimes { <bit>: unix } } }
#
# ID <-> BUCKET/BIT CONVENTION:  achievement_id (protocol eMsg 5466) =
# bucket*32 + bit. The snapshot command derives ids this way; rebuild rechecks
# bucket/bit against the schema when provided (warns if a bucket is missing).
# ============================================================================

import struct
import sys
import json
import argparse
from datetime import datetime, timezone, timedelta

# Italian local time (CEST) for display; change if needed.
LOCAL_TZ = timezone(timedelta(hours=2))

MIRROR_FORMAT = 'aether-achievement-mirror-v1'


# ---------------------------------------------------------------------------
# Parsing binary VDF
# ---------------------------------------------------------------------------

def parse_bin_vdf(buf, pos=0):
    """Parse binary VDF. Returns (list of (key, value), new_pos)."""
    out = []
    while pos < len(buf):
        t = buf[pos]
        pos += 1
        if t == 0x08:
            return out, pos
        end = buf.index(b'\x00', pos)
        key = buf[pos:end].decode('utf-8', 'replace')
        pos = end + 1
        if t == 0x00:
            val, pos = parse_bin_vdf(buf, pos)
            out.append((key, dict(val)))
        elif t == 0x01:
            end = buf.index(b'\x00', pos)
            out.append((key, ('str', bytes(buf[pos:end]))))
            pos = end + 1
        elif t == 0x02:
            val = struct.unpack('<i', buf[pos:pos + 4])[0]
            out.append((key, ('int', val)))
            pos += 4
        elif t == 0x03:
            val = struct.unpack('<f', buf[pos:pos + 4])[0]
            out.append((key, ('flt', val)))
            pos += 4
        else:
            raise ValueError(f'unknown type 0x{t:02x} at position {pos - 1}')
    return out, pos


def fmt_time(unix_ts):
    try:
        return datetime.fromtimestamp(unix_ts, LOCAL_TZ).strftime('%d/%m/%Y %H:%M:%S')
    except (OverflowError, OSError, ValueError):
        return '?'


# ---------------------------------------------------------------------------
# Reading: schema / stats bin / snapshot json
# ---------------------------------------------------------------------------

def parse_schema(data):
    """Returns ({bucket: {bit: (api, english)}}, meta)."""
    root = dict(parse_bin_vdf(data)[0])
    appid_key = next(iter(root))
    app = root[appid_key]
    schema = {}
    for bucket, bv in app.get('stats', {}).items():
        bits = bv.get('bits', {})
        if not isinstance(bits, dict):
            continue
        for bit, v in bits.items():
            if not isinstance(v, dict):
                continue
            nm = v.get('name', ('str', b'?'))[1]
            eng = ''
            disp = v.get('display', {})
            if isinstance(disp, dict):
                names = disp.get('name', {})
                if isinstance(names, dict):
                    eng_b = names.get('english', ('str', b''))[1]
                    eng = eng_b.decode('utf-8', 'replace')
            schema.setdefault(int(bucket), {})[int(bit)] = (
                nm.decode('utf-8', 'replace') if isinstance(nm, bytes) else str(nm), eng)
    meta = {'appid': appid_key}
    for k, attr in (('gamename', 'gamename'), ('version', 'version')):
        if k in app:
            val = app[k][1]
            meta[attr] = val.decode('utf-8', 'replace') if isinstance(val, bytes) else val
    return schema, meta


def parse_stats(data):
    """Returns (crc, pending, {bucket: {'data': int, 'times': {bit: unix}}})."""
    root = dict(parse_bin_vdf(data)[0])
    cache = root.get('cache', {})
    crc = cache.get('crc', ('int', 0))[1]
    pending = cache.get('PendingChanges', ('int', 0))[1]
    buckets = {}
    for bucket, bv in cache.items():
        if not isinstance(bv, dict) or 'AchievementTimes' not in bv:
            continue
        bitfield = bv.get('data', ('int', 0))[1] & 0xFFFFFFFF
        times = {}
        for bit, tv in bv['AchievementTimes'].items():
            if isinstance(tv, tuple) and tv[0] == 'int':
                times[int(bit)] = tv[1]
        buckets[int(bucket)] = {'data': bitfield, 'times': times}
    return crc, pending, buckets


def parse_snapshot(data):
    """Returns (meta, [(id, unlock_time, bucket, bit), ...]) from a snapshot JSON."""
    obj = json.loads(data.decode('utf-8'))
    if str(obj.get('_format', '')).startswith('aether-achievement-mirror'):
        entries = obj.get('achievements', [])
    elif 'achievements' in obj:
        entries = obj['achievements']
    else:
        raise ValueError('JSON without "achievements" key')
    rows = []
    for e in entries:
        eid = int(e['id'])
        t = int(e.get('unlock_time', 0))
        bucket = e.get('bucket')
        bit = e.get('bit')
        if bucket is None or bit is None:
            bucket, bit = eid // 32, eid % 32
        rows.append((eid, t, int(bucket), int(bit)))
    return obj, rows


# ---------------------------------------------------------------------------
# Writing: snapshot json / bin
# ---------------------------------------------------------------------------

def now_iso():
    return datetime.now(LOCAL_TZ).strftime('%Y-%m-%dT%H:%M:%S')


def snapshot_json(appid, account_id, steamid64, entries):
    """entries: [(id, unlock_time)] sorted by id."""
    out = ['{',
           f'  "_format": "{MIRROR_FORMAT}",',
           '  "_note": "snapshot generated by AetherDLL; rebuild the .bin with Tools/achievement_decoder.py rebuild",',
           f'  "appid": {appid},',
           f'  "account_id": {account_id},',
           f'  "steamid64": {steamid64},',
           f'  "updated": "{now_iso()}",',
           '  "achievements": [']
    for i, (eid, t) in enumerate(entries):
        out.append(f'    {{"id": {eid}, "unlock_time": {t}, "unlocked_at": "{fmt_time(t)}", '
                   f'"bucket": {eid // 32}, "bit": {eid % 32}}}' + (',' if i + 1 < len(entries) else ''))
    out.append('  ]')
    out.append('}')
    return ('\n'.join(out) + '\n').encode('utf-8')


def build_bin(buckets, pending=1):
    """buckets: {bucket: {bit: unix_time}} -> bytes of the UserGameStats file."""
    out = bytearray()
    out += b'\x00cache\x00'
    out += b'\x02crc\x00' + struct.pack('<i', 0)
    out += b'\x02PendingChanges\x00' + struct.pack('<i', pending)
    for bucket in sorted(buckets):
        times = buckets[bucket]
        bitfield = 0
        for bit in times:
            bitfield |= (1 << bit)
        signed = bitfield if bitfield < (1 << 31) else bitfield - (1 << 32)
        out += b'\x00' + str(bucket).encode() + b'\x00'
        out += b'\x02data\x00' + struct.pack('<i', signed)
        out += b'\x02state\x00' + struct.pack('<i', 2)
        out += b'\x00AchievementTimes\x00'
        for bit in sorted(times):
            out += b'\x02' + str(bit).encode() + b'\x00' + struct.pack('<i', times[bit])
        out += b'\x08'   # closes AchievementTimes
        out += b'\x08'   # closes the bucket
    out += b'\x08'       # closes cache
    out += b'\x08'       # closes the root dictionary (written by Steam itself)
    return bytes(out)


# ---------------------------------------------------------------------------
# Commands
# ---------------------------------------------------------------------------

def cmd_decode(paths):
    schema = {}
    schema_meta = {}
    items = []   # ('stats_bin', path, data) | ('snapshot', path, data)

    for path in paths:
        data = open(path, 'rb').read()
        try:
            entries, _ = parse_bin_vdf(data)
            keys = [k for k, _ in entries]
        except ValueError as e:
            if path.lower().endswith('.json'):
                try:
                    meta, rows = parse_snapshot(data)
                    items.append(('snapshot', path, (meta, rows)))
                    print(f'[snapshot] {path}: {len(rows)} achievement '
                          f'(appid {meta.get("appid", "?")}, account {meta.get("account_id", "?")})')
                    continue
                except Exception as je:
                    print(f'[!] {path}: parse failed ({e} / {je})')
                    continue
            print(f'[!] {path}: parse failed ({e})')
            continue
        if len(keys) == 1 and keys[0].isdigit():          # schema: radice = appid
            schema, schema_meta = parse_schema(data)
            print(f'[schema] {path}: appid {schema_meta.get("appid")}, '
                  f'internal game name "{schema_meta.get("gamename", "?")}", '
                  f'{sum(len(v) for v in schema.values())} achievement in {len(schema)} bucket')
        elif 'cache' in keys:                             # file UserGameStats
            items.append(('stats_bin', path, data))
        else:
            print(f'[!] {path}: unrecognized structure (keys: {keys[:3]})')

    for kind, path, payload in items:
        print()
        print(f'===== {path} =====')
        if kind == 'stats_bin':
            crc, pending, buckets = parse_stats(payload)
            print(f'  crc = {crc}, PendingChanges = {pending} '
                  f'({"local changes NOT confirmed by the server" if pending else "all confirmed"})')
            print(f'  bucket: {sorted(buckets.keys())}')
            rows = []
            total = 0
            for bucket, info in sorted(buckets.items()):
                names = schema.get(bucket, {})
                for bit, t in sorted(info['times'].items()):
                    rows.append((t, bucket, bit, bucket * 32 + bit, names.get(bit, ('?', '(bit not defined in schema)'))))
                total += bin(info['data']).count('1')
            print(f'  achievements recorded in this file: {total}')
        else:
            meta, rows_raw = payload
            print(f'  AetherDLL backup snapshot — account {meta.get("account_id")}, '
                  f'appid {meta.get("appid")}, updated {meta.get("updated", "?")}')
            rows = []
            for eid, t, bucket, bit in rows_raw:
                names = schema.get(bucket, {})
                rows.append((t, bucket, bit, eid, names.get(bit, ('?', '(bucket/bit not in schema)'))))
            print(f'  achievements recorded in this file: {len(rows)}')

        if rows:
            print('  --- Unlock timeline (id = bucket*32+bit) ---')
            for t, bucket, bit, eid, (api, eng) in sorted(rows):
                print(f'  {fmt_time(t)}  bucket={bucket:3d} bit={bit:2d} id={eid:5d}  {api:34s} "{eng}"')
        else:
            print('  NO achievements in this file (empty cache).')
    return 0


def cmd_snapshot(args):
    data = open(args.bin, 'rb').read()
    crc, pending, buckets = parse_stats(data)

    # account/appid from the UserGameStats_<account>_<appid>.bin filename
    import os
    base = os.path.basename(args.bin)
    account_id, appid = '?', '?'
    if base.startswith('UserGameStats_'):
        parts = base[len('UserGameStats_'):].split('.')[0].split('_')
        if len(parts) >= 2:
            account_id, appid = parts[0], parts[1]

    entries = []
    for bucket, info in sorted(buckets.items()):
        for bit, t in sorted(info['times'].items()):
            entries.append((bucket * 32 + bit, t))
    entries.sort()
    steamid64 = 0x0110000100000000 | int(account_id) if str(account_id).isdigit() else 0
    blob = snapshot_json(appid, account_id, steamid64, entries)
    if args.output:
        open(args.output, 'wb').write(blob)
        print(f'[ok] wrote {len(entries)} achievements -> {args.output}')
    else:
        sys.stdout.write(blob.decode('utf-8'))
    return 0


def cmd_rebuild(args):
    meta, rows = parse_snapshot(open(args.json_file, 'rb').read())
    schema = {}
    if args.schema:
        schema, _ = parse_schema(open(args.schema, 'rb').read())

    buckets = {}
    for eid, t, bucket, bit in rows:
        if schema and bucket not in schema:
            print(f'[!] WARN: bucket {bucket} (from id {eid}) not in schema — writing anyway.')
        buckets.setdefault(bucket, {})[bit] = t

    blob = build_bin(buckets, pending=args.pending)
    out = args.output or ('UserGameStats_%s_%s.bin' % (meta.get('account_id', 'X'), meta.get('appid', 'X')))
    open(out, 'wb').write(blob)
    print(f'[ok] rebuilt {out}: {len(rows)} achievements in {len(buckets)} buckets, '
          f'PendingChanges={args.pending}')
    print('     To restore: close Steam, copy the file into <steam>\\appcache\\stats\\, restart Steam.')
    return 0


def main(argv):
    ap = argparse.ArgumentParser(description='Steam achievement cache decoder/encoder + AetherDLL backup tool')
    sub = ap.add_subparsers(dest='command')

    p_dec = sub.add_parser('decode', help='decode bin/schema/json files (default)')
    p_dec.add_argument('files', nargs='+')

    p_snap = sub.add_parser('snapshot', help='UserGameStats .bin -> snapshot JSON (AetherDLL backup format)')
    p_snap.add_argument('bin')
    p_snap.add_argument('--schema')
    p_snap.add_argument('-o', '--output')

    p_reb = sub.add_parser('rebuild', help='snapshot JSON -> UserGameStats .bin')
    p_reb.add_argument('json_file')
    p_reb.add_argument('--schema', help='schema .bin to validate bucket/bit (recommended)')
    p_reb.add_argument('--pending', type=int, default=1, help='PendingChanges value written into the .bin (default 1)')
    p_reb.add_argument('-o', '--output')

    # Default mode: without a subcommand, every argument is a file to decode.
    args = ap.parse_args(argv[1:] if argv[1:2] and argv[1] in ('decode', 'snapshot', 'rebuild')
                         else ['decode'] + argv[1:])

    if args.command == 'decode':
        return cmd_decode(args.files)
    if args.command == 'snapshot':
        return cmd_snapshot(args)
    if args.command == 'rebuild':
        return cmd_rebuild(args)
    ap.print_help()
    return 1


if __name__ == '__main__':
    sys.exit(main(sys.argv))
