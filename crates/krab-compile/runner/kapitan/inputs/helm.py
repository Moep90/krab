"""`HelmChart`: a helm chart rendered into kadet objects.

    class MyChart(HelmChart): ...
    for obj in HelmChart(chart_dir="charts/x", helm_values={...}).root.values(): ...

The chart is templated by krab's compiler, which validates the parameters,
records the chart files as dependencies of the target, parses the output
and caches identical renders across compiles.
"""

import tempfile

import yaml
from kadet import BaseModel, BaseObj

from kapitan import runtime
from kapitan.errors import CompileError, HelmTemplateError

HELM_DENIED_FLAGS = {"dry-run", "generate-name", "help", "output-dir", "show-only"}
HELM_DEFAULT_FLAGS = {"--include-crds": True, "--skip-tests": True}


def _helm_str_representer(dumper, data):
    """Quote strings helm's Go YAML parser would read as numbers ("03190301")."""
    style = None
    if data and len(data) > 1 and data.isdigit() and (data[0] == "0" or len(data) > 6):
        style = "'"
    return dumper.represent_scalar("tag:yaml.org,2002:str", data, style=style)


def write_helm_values_file(helm_values: dict):
    """Dump helm values into a temporary YAML file and return its path."""
    _, helm_values_file = tempfile.mkstemp(".helm_values.yml", text=True)
    with open(helm_values_file, "w") as fp:
        dumper = yaml.SafeDumper
        dumper.add_representer(str, _helm_str_representer)
        yaml.dump(helm_values, fp, Dumper=dumper)
    return helm_values_file


def _render(chart_dir, helm_path, helm_params, helm_values_file, helm_values_files, helm_flags, parse):
    return runtime.render_helm(
        {
            "chart_dir": chart_dir,
            "helm_path": helm_path,
            "helm_params": dict(helm_params or {}),
            "helm_values_file": helm_values_file,
            "helm_values_files": list(helm_values_files or []),
            "helm_flags": dict(helm_flags) if helm_flags is not None else None,
            "parse": parse,
        }
    )


def render_chart(chart_dir, output_path, helm_path, helm_params, helm_values_file, helm_values_files, helm_flags=None):
    """`helm template` of the chart at `chart_dir`, as kapitan's function of
    the same name: returns `(output, error_message)`. Only `output_path="-"`
    (the rendered chart as text) is supported here; writing a directory is
    what the `helm` input type does."""
    if output_path != "-":
        raise CompileError("render_chart: krab renders charts to a string (output_path='-') inside kadet components")
    try:
        return _render(chart_dir, helm_path, helm_params, helm_values_file, helm_values_files, helm_flags, False)["output"], ""
    except HelmTemplateError as e:
        return "", str(e)


class HelmChart(BaseModel):
    """Renders the chart at `chart_dir` and stores the objects in `root`,
    keyed `<name>-<kind>`. Raises `HelmTemplateError` when helm fails."""

    chart_dir: str
    helm_params: dict = {}
    helm_values: dict = {}
    helm_path: str = None

    def new(self):
        for obj in self.load_chart():
            self.root[f"{obj['metadata']['name'].lower()}-{obj['kind'].lower().replace(':', '-')}"] = BaseObj.from_dict(obj)

    def load_chart(self):
        """The rendered documents, parsed."""
        helm_values_file = write_helm_values_file(self.helm_values) if self.helm_values else None
        return _render(self.chart_dir, self.helm_path, self.helm_params, helm_values_file, None, None, True)["docs"]
