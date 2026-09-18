"""kapitan's exception hierarchy. `CompileError` is what a component raises
(and what kadet raises through `ABORT_EXCEPTION_TYPE`); the others exist so
code that catches them keeps importing."""


class KapitanError(Exception):
    """generic kapitan error"""


class CompileError(KapitanError):
    """compile error"""


class InventoryError(KapitanError):
    """inventory error"""


class InventoryValidationError(InventoryError):
    """inventory validation error"""


class InvalidTargetError(InventoryError):
    """unknown target"""


class SecretError(KapitanError):
    """secrets error"""


class RefError(KapitanError):
    """ref error"""


class RefBackendError(KapitanError):
    """ref backend error"""


class RefFromFuncError(KapitanError):
    """ref from func error"""


class RefHashMismatchError(KapitanError):
    """ref hash mismatch error"""


class HelmBindingUnavailableError(KapitanError):
    """helm input is used when the binding is not available"""


class HelmFetchingError(KapitanError):
    """fetching a helm chart failed"""


class HelmTemplateError(KapitanError):
    """`helm template` failed"""


class GitSubdirNotFoundError(KapitanError):
    """git dependency subdir not found error"""


class GitFetchingError(KapitanError):
    """repo not found and/or permission error"""


class RequestUnsuccessfulError(KapitanError):
    """request error"""


class KubernetesManifestValidationError(KapitanError):
    """kubernetes manifest schema validation error"""


class KustomizeTemplateError(KapitanError):
    """kustomize template failed"""


class CuelangTemplateError(KapitanError):
    """cuelang template failed"""


class OCIFetchingError(KapitanError):
    """fetching an OCI artifact failed"""
