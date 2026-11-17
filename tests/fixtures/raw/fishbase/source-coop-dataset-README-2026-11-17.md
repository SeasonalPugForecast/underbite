---
license: cc-by-nc-4.0
---

# Dataset Card for FishBase

<!-- Provide a quick summary of the dataset. -->

Snapshots of FishBase data tables used by the [rOpenSci package `rfishbase`](https://github.com/ropensci/rfishbase) and the FishBase API.

## Dataset Details

### Dataset Description

<!-- Provide a longer summary of what this dataset is. -->


- **Curated by:** Carl Boettiger
- **License:** CC-BY-NC, FishBase.org Team

### Dataset Sources 

<https://fishbase.org>, <https://sealifebase.org>, via exclusive export of backend MySQL databases.

## Uses

<!-- Address questions around how the dataset is intended to be used. -->

### Direct Use

This data is provided to facilitate access to fishbase statistics through `rfishbase`.
Curation has evolved over the past decade and now provides basically direct access to versioned snapshots of the fishbase backend data.


### Out-of-Scope Use

Reuse of this data is not licensed for commerical applications. Developers should contact the FishBase team directly. See <https://fishbase.org> for details.

## Dataset Structure

Data provided here are raw parquet exports from the MySQL database backend powering <https://fishbase.org> and its sister site, <https://sealifebase.org>,
laid out as `{fb,slb}/v<version>/parquet/<table>.parquet` (`fb` for FishBase, `slb` for SeaLifeBase).
Please note that there are many tables here which have evolved over decades and were not developed with the intent of ever supporting direct access by end users. 
The data structure can be highly unintuitive and common tasks may require joins across multiple tables.  These files represent a snapshot in time as indicated
by the version numbers (e.g. v24.07 is data snapshot dated to July of 2024).  The two servers are versioned independently, since their
dumps do not always arrive together: the current releases are `fb/v26.07` and `slb/v26.04`.

## Dataset Creation

Dataset creation relies on ingesting a MySQL dump provided roughly annually from the FishBase team to this maintainer.  The current process imports
the dump into a scratch MariaDB server, attaches it from `duckdb` via the `mysql` extension, and exports one zstd-compressed parquet file per table.
The scripts and step-by-step notes live in the `rfishbase` repository under
[`data-raw/`](https://github.com/ropensci/rfishbase/tree/master/data-raw).



### Source Data

See <https://fishbase.org> for details.


