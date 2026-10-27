'''
tetron-tui main spec
'''

from libspec import Spec
from . import packaging, constraints

class MainSpec(Spec):
    def modules(self):
        return [packaging, constraints]
