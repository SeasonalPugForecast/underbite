Attributes {
  time {
    String _CoordinateAxisType "Time";
    Float64 actual_range 1.7632944e+9, 1.7944848e+9;
    String axis "T";
    String calendar "gregorian";
    String ioos_category "Time";
    String long_name "Time Baseline";
    String source_name "time_baseline";
    String standard_name "time";
    String time_origin "01-JAN-1970 00:00:00";
    String units "seconds since 1970-01-01T00:00:00Z";
  }
  altitude {
    String _CoordinateAxisType "Height";
    String _CoordinateZisPositive "up";
    Float64 actual_range 0.0, 0.0;
    String axis "Z";
    String ioos_category "Location";
    String long_name "Altitude";
    String positive "up";
    String standard_name "altitude";
    String units "m";
  }
  latitude {
    String _CoordinateAxisType "Lat";
    Float64 actual_range -89.75625000000002, 89.75625000000001;
    String axis "Y";
    String ioos_category "Location";
    String long_name "Latitude";
    String source_name "rows";
    String standard_name "latitude";
    String units "degrees_north";
  }
  longitude {
    String _CoordinateAxisType "Lon";
    Float64 actual_range -180.01875, 180.01874999999998;
    String axis "X";
    String ioos_category "Location";
    String long_name "Longitude";
    String source_name "cols";
    String standard_name "longitude";
    String units "degrees_east";
  }
  kd_490 {
    UInt32 _ChunkSizes 1, 1, 902, 1808;
    Float32 _FillValue NaN;
    Float64 colorBarMaximum 4.0;
    Float64 colorBarMinimum 0.0;
    String colorBarPalette "WhiteRedBlack";
    String cw_atmospheric_correction "NOAA MECB BMW Atmospheric Correction";
    Int32 cw_fraction_digits 14;
    String cw_processing_algorithm "NOAA MECB Kd_490 Algorithm";
    String cw_product_status "Experimental";
    String display_scale "log";
    String grid_mapping "coord_ref";
    String ioos_category "Optical Properties";
    String long_name "Diffuse Attenuation Coefficient at 490 nm";
    Float32 missing_value NaN;
    String units "m^-1";
    Float32 valid_max 32.0;
    Float32 valid_min 0.016;
  }
  graphics {
    UInt32 _ChunkSizes 1, 1, 1193, 2393;
    Byte _FillValue 0;
    String _Unsigned "false";
    Float64 colorBarMaximum 20.0;
    Float64 colorBarMinimum 0.0;
    Int32 cw_fraction_digits 0;
    String grid_mapping "coord_ref";
    String ioos_category "Optical Properties";
    String long_name "graphics overlay planes";
    Byte missing_value 0;
  }
  NC_GLOBAL {
    String cdm_data_type "Grid";
    String Conventions "CF-1.6, COARDS, ACDD-1.3";
    String creator_email "coastwatch.info@noaa.gov";
    String creator_name "NOAA CoastWatch";
    String creator_type "group";
    String creator_url "https://coastwatch.noaa.gov/";
    String date_created "2026-11-14T01:46:37Z";
    Float64 Easternmost_Easting 180.01874999999998;
    Float64 geospatial_lat_max 89.75625000000001;
    Float64 geospatial_lat_min -89.75625000000002;
    Float64 geospatial_lat_resolution 0.03750000000000001;
    String geospatial_lat_units "degrees_north";
    Float64 geospatial_lon_max 180.01874999999998;
    Float64 geospatial_lon_min -180.01875;
    Float64 geospatial_lon_resolution 0.037500000000000006;
    String geospatial_lon_units "degrees_east";
    Float64 geospatial_vertical_max 0.0;
    Float64 geospatial_vertical_min 0.0;
    String geospatial_vertical_positive "up";
    String geospatial_vertical_units "m";
    String history "Processed with NOAA MSL12
2026-11-16T23:50:41Z (local files)
2026-11-16T23:50:41Z https://coastwatch.noaa.gov/erddap/griddap/noaacwNPPVIIRSkd490Daily.das";
    String infoUrl "https://coastwatch.noaa.gov/thredds/dodsC/CoastWatch/VIIRS/kd490/Daily/WW00.html";
    String institution "NOAA NESDIS CoastWatch";
    String instrument "VIIRS";
    String keywords "altitude, applications, attenuation, baseline, center, coefficient, data, diffuse, environmental, graphics, imager, imager/radiometer, imaging, information, infrared, kd490, kd_490, latitude, longitude, mean, national, nesdis, noaa, npp, optical, optical properties, orbiting, overlay, partnership, planes, polar, polar-orbiting, properties, radiometer, research, satellite, service, suite, time, viirs, visible, ww00";
    String keywords_vocabulary "NASA Global Change Master Directory (GCMD) Science Keywords";
    String license "The data may be used and redistributed for free but is not intended
for legal use, since it may contain inaccuracies. Neither the data
Contributor, ERD, NOAA, nor the United States Government, nor any
of their employees or contractors, makes any warranty, express or
implied, including warranties of merchantability and fitness for a
particular purpose, or assumes any legal liability for the accuracy,
completeness, or usefulness, of this information.";
    String naming_authority "gov.noaa.nesdis.star/socd/mecb/color/";
    Float64 Northernmost_Northing 89.75625000000001;
    String OBSERVED_PROPERTY "kd_490";
    String platform "S-NPP";
    String processing_level "L2";
    String project "Ocean Color Science Team (NOAA/NESDIS/STAR/OCST)";
    String publisher_email "coastwatch.info@noaa.gov; ncei.info@noaa.gov";
    String publisher_name "NOAA CoastWatch; National Centers for Environmental Information (NCEI)";
    String publisher_url "https://coastwatch.noaa.gov; https://www.ncei.noaa.gov/";
    String source "S-NPP_VIIRS_kd490";
    String sourceUrl "(local files)";
    Float64 Southernmost_Northing -89.75625000000002;
    String standard_name_vocabulary "CF Standard Name Table v29";
    String summary "Visible and Infrared Imager/Radiometer Suite/Suomi-NPP (VIIRS) Level-3 (WW00), Diffuse Attenuation Coefficient at 490nm (kd490), Global, Daily, processed by NOAA.  EXPERIMENTAL.";
    String testOutOfDate "now-2days";
    String time_coverage_end "2026-11-12T12:00:00Z";
    String time_coverage_start "2025-11-16T12:00:00Z";
    String title "Kd490, NOAA S-NPP VIIRS, Near Real-Time, Global 4km, Level 3, 2017 to 2026-11-02,  Daily";
    Float64 Westernmost_Easting -180.01875;
  }
}
