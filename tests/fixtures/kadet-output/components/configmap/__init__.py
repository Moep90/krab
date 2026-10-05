from kapitan.inputs.kadet import BaseObj, inventory


def main(input_params):
    obj = BaseObj()
    obj.root.cm = {
        "apiVersion": "v1",
        "kind": "ConfigMap",
        "metadata": {"name": "demo"},
        "data": {"settings.yaml": inventory().parameters.payload},
    }
    return obj
