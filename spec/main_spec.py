'''
tetron-tui main spec
'''

from libspec import Spec
from . import packaging, constraints, selected_peer

class MainSpec(Spec):
    def modules(self):
        return [packaging, constraints, selected_peer]
