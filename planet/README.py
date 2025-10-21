# Planet calculation

## Databases

The databases used are:

* PostgreSQL to store system/planet data
* DuckDB to store calculated positions


### DuckDB

DuckDB permit to have very many data and reduce the size on disk (for example 500 Mio to 30 Mio).

To install in local, do:

```bash
curl https://install.duckdb.org | sh
```

## Scripts

### Import system from JSON

The script `import_system_from_json.py` permit to import a system from a JSON file into the PostgreSQL database.

Usage:

```bash
python import_system_from_json.py systems/system.json SystemName
```

### Calculate positions

The script `kepler_positions.py` permit to calculate the positions of the planets in a system and store them into a DuckDB database.

Usage:

```bash
python kepler_positions.py SystemName
```

**This script not use yet the PostgreSQL database to get data**

