'''
The selected peer is visible wherever an action targets it (USER bug 2026-11-04).
'''

from .err import Req


class SelectedPeerShown(Req):
    """REQUIREMENT-ID: TUI-PEER-001

    The History and Admin tabs show the selected peer on their first line:
    hostname, mesh IP and the first 8 characters of its key, or "no peer
    selected". The kick and grant-admin confirmations name the peer the
    same way (hostname, mesh IP, short key) and the network, so the user
    always sees whom the action hits.
    """
