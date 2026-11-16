Attributes {
  time {
    String _CoordinateAxisType "Time";
    Float64 actual_range 1.646136e+9, 1.794744e+9;
    String axis "T";
    String ioos_category "Time";
    String long_name "Centered Time";
    String standard_name "time";
    String time_origin "01-JAN-1970 00:00:00";
    String units "seconds since 1970-01-01T00:00:00Z";
  }
  latitude {
    String _CoordinateAxisType "Lat";
    Float32 _FillValue NaN;
    Float32 actual_range -89.97917, 89.97916;
    String axis "Y";
    String ioos_category "Location";
    String long_name "Latitude";
    String standard_name "latitude";
    String units "degrees_north";
    Float32 valid_max 90.0;
    Float32 valid_min -90.0;
  }
  longitude {
    String _CoordinateAxisType "Lon";
    Float32 _FillValue NaN;
    Float32 actual_range -179.9792, 179.9792;
    String axis "X";
    String ioos_category "Location";
    String long_name "Longitude";
    String standard_name "longitude";
    String units "degrees_east";
    Float32 valid_max 180.0;
    Float32 valid_min -180.0;
  }
  Kd_490 {
    Float32 _FillValue NaN;
    Float64 colorBarMaximum 10.0;
    Float64 colorBarMinimum -10.0;
    String ioos_category "Optical Properties";
    String long_name "Diffuse attenuation coefficient at 490 nm, KD2 algorithm";
    String units "m-1";
    Float32 valid_max 6.0;
    Float32 valid_min 0.01;
  }
  NC_GLOBAL {
    String _lastModified "2026-11-16T12:46:39.000Z";
    String cdm_data_type "Grid";
    String Conventions "CF-1.6 ACDD-1.3, COARDS";
    String creator_email "data@oceancolor.gsfc.nasa.gov";
    String creator_name "NASA/GSFC/OBPG";
    String creator_type "group";
    String creator_url "https://oceandata.sci.gsfc.nasa.gov";
    String date_created "2026-11-16T12:46:39.000Z";
    Float64 Easternmost_Easting 179.9792;
    Float64 geospatial_lat_max 89.97916;
    Float64 geospatial_lat_min -89.97917;
    Float64 geospatial_lat_resolution 0.04166666589488307;
    String geospatial_lat_units "degrees_north";
    Float64 geospatial_lon_max 179.9792;
    Float64 geospatial_lon_min -179.9792;
    Float64 geospatial_lon_resolution 0.041666674383609215;
    String geospatial_lon_units "degrees_east";
    String history "Files downloaded daily from https://oceandata.sci.gsfc.nasa.gov/MODIS-Aqua/L3SMI to NOAA SWFSC ERD (erd.data@noaa.gov) ERDDAP adds the time dimension.
2026-11-16T23:46:28Z (local files)
2026-11-16T23:46:28Z https://coastwatch.pfeg.noaa.gov/erddap/griddap/erdMH1kd4901day_R2022NRT.das";
    String id "R2022.0.3/L3/AQUA_MODIS.20261115.L3b.DAY.KD.NRT.nc";
    String identifier_product_doi "10.5067/AQUA/MODIS/L3M/KD/2022";
    String identifier_product_doi_authority "https://dx.doi.org";
    String infoUrl "https://oceandata.sci.gsfc.nasa.gov";
    String institution "NASA/GSFC OBPG";
    String instrument "MODIS";
    String keywords "algorithm, aqua, attenuation, attenuation/transmission, biology, center, coefficient, color, data, diffuse, earth, Earth Science > Oceans > Ocean Optics > Attenuation/Transmission, flight, goddard, group, gsfc, image, imaging, kd2, Kd_490, L3, level, level-3, mapped, moderate, modis, modisa, nasa, obpg, ocean, oceans, optical, optical properties, optics, processing, properties, resolution, science, smi, space, spectroradiometer, standard, transmission";
    String keywords_vocabulary "GCMD Science Keywords";
    String l2_flag_names "ATMFAIL,LAND,HILT,HISATZEN,STRAYLIGHT,CLDICE,COCCOLITH,LOWLW,CHLWARN,CHLFAIL,NAVWARN,MAXAERITER,ATMWARN,HISOLZEN,NAVFAIL,FILTER,HIGLINT";
    String license "https://science.nasa.gov/earth-science/earth-science-data/data-information-policy/";
    String map_projection "Equidistant Cylindrical";
    String measure "Mean";
    String naming_authority "gov.nasa.gsfc.sci.oceandata";
    Float64 Northernmost_Northing 89.97916;
    String platform "Aqua";
    String processing_level "L3 Mapped";
    String processing_version "R2022.0.3";
    String product_name "AQUA_MODIS.20261115.L3m.DAY.KD.Kd_490.4km.NRT.nc";
    String project "Ocean Biology Processing Group (NASA/GSFC/OBPG)";
    String publisher_email "erd.data@noaa.gov";
    String publisher_name "NOAA NMFS SWFSC ERD";
    String publisher_type "institution";
    String publisher_url "https://www.pfeg.noaa.gov";
    String source "satellite observations from MODIS-Aqua";
    String sourceUrl "(local files)";
    Float64 Southernmost_Northing -89.97917;
    String spatialResolution "4.638312 km";
    String standard_name_vocabulary "CF Standard Name Table v36";
    String summary "Moderate Resolution Imaging Spectroradiometer on Aqua (MODISA) Level-3 Standard Mapped Image";
    String temporal_range "day";
    String time_coverage_end "2026-11-15T12:00:00Z";
    String time_coverage_start "2022-03-01T12:00:00Z";
    String title "Diffuse Attenuation K490, Aqua MODIS, NPP, L3SMI, Global, 4km, R2022 NRT, 2003-present (1 Day Composite)";
    Float64 Westernmost_Easting -179.9792;
  }
}
