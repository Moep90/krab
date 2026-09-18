"""The kapitan defaults component code can observe."""

import os

KADET_COMPONENT_MODULE_PREFIX = "kadet_component_"
# `render_jinja2_file` loads filters from this file, relative to the
# compile's working directory, when it exists.
DEFAULT_JINJA2_FILTERS_PATH: str = os.path.join("lib", "jinja2_filters.py")
