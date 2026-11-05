'''
The join field accepts a pasted invite, a file path or a URL (USER 2026-11-05).
Invite text from a file or a URL is checked by tetron-proto
`invite::read_invite` (core v0.15.2, JOIN-INPUT-001), the same rules as
`tetron join --invite-file`.
'''

from .err import Req


class JoinInputKinds(Req):
    """REQUIREMENT-ID: TUI-JOIN-INPUT-001

    The join field (J) classifies its trimmed text: starting with
    http:// or https:// (any case) is a URL (TUI-JOIN-URL-001); otherwise
    text holding '/', '.' or a leading '~' is a file path
    (TUI-JOIN-FILE-001), since base58 invite codes contain none of them;
    anything else is a pasted invite, joined as before. The input box
    title and the help line say "invite, file or URL".
    """


class JoinByFile(Req):
    """REQUIREMENT-ID: TUI-JOIN-FILE-001

    A file path is read with `invite::read_invite`, a leading `~/` is
    $HOME. Errors name the path, never the content. The file is never
    deleted or changed. The invite then goes through IPC Join as a pasted
    one.
    """


class JoinByUrl(Req):
    """REQUIREMENT-ID: TUI-JOIN-URL-001

    A URL is fetched with curl and the fixed flag set (mesh join plan item
    19): `--proto =https -fsS --max-filesize 8192 --max-time 15`, no
    redirects. An http:// URL first asks for confirmation ("plain HTTP,
    anyone on the path can read the invite") and then fetches with
    `--proto =http,https`. The body is checked with `invite::read_invite`
    and then joined as a pasted invite. Errors name the step that failed
    (curl missing, curl exit and message, body check) and never contain
    the invite.
    """


class DebDependsCurl(Req):
    """REQUIREMENT-ID: DEB-007

    packaging/deb.toml declares Depends curl (join by URL).
    """
