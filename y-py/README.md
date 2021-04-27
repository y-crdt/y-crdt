# y-py

y-py is a Pythong binding to Yrs using Maturin, using pyo3.

Reference : 
https://docs.rs/crate/maturin/0.10.3


### install maturin 

maturin is necessary to build y-py
```
pip install maturin
```

### Build y-py : 
- Build and install locally as a python module : ``` maturin develop ```
- Build the wheels and stores them in `target/wheels` : ```maturin build```



### use y-py :

```
import y_py

# shows all the functions available in module y_py
print(dir(y_py))

# To be completed with working examples

```



