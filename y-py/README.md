# Y-Py

Y-Py is a Python binding for Y-CRDT. It provides distributed data types that enable real-time collaboration between devices. Y-Py can sync data with any other platform that has a Y-CRDT binding, allowing for seamless cross-domain communication. The library is a thin wrapper around Yrs, taking advantage of the safety and performance of Rust.

## Installation
```
pip install y-py
```

## Getting Started
Y-Py provides many of the same shared data types as [Yjs](https://docs.yjs.dev/). All objects are shared within a `YDoc` and get modified within a transaction block. 

```python
import y_py as Y

d1 = Y.YDoc()
# Create a new YText object in the YDoc
text = d1.get_text('test')
# Start a transaction in order to update the text
with d1.begin_transaction() as txn:
    # Add text contents
    text.push(txn, "hello world!")

# Create another document
d2 = Y.YDoc()
# Share state with the original document
state_vector = Y.encode_state_vector(d2)
diff = Y.encode_state_as_update(d1, state_vector)
Y.apply_update(d2, diff)

with d2.begin_transaction() as txn: 
    value = d2.get_text('test').to_string(txn)

assert value == "hello world!"
```

## Development Setup
0. Install Rust Nightly and Python
1. Install `maturin` in order to build Y-Py 
```
pip install maturin
```
2. Create a development build of the library
``` maturin develop ```

## Tests
All tests are located in `/tests`. You can run them with `pytest`.
```
pytest
```

## Build Y-Py :
Build the library as a wheel and store them in `target/wheels` : 
```
maturin build
```





