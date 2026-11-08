Attributes {
  time {
    UInt32 _ChunkSizes 1024;
    String _CoordinateAxisType "Time";
    Float64 actual_range 1.7860644e+9, 1.7941248e+9;
    String axis "T";
    String bounds "time_bnds";
    String calendar "gregorian";
    String ioos_category "Time";
    String long_name "Time";
    String standard_name "time";
    String time_origin "01-JAN-1970 00:00:00";
    String units "seconds since 1970-01-01T00:00:00Z";
  }
  latitude {
    String _CoordinateAxisType "Lat";
    Float32 actual_range 21.73596, 46.49442;
    String axis "Y";
    String ioos_category "Location";
    String long_name "Latitude";
    String standard_name "latitude";
    String units "degrees_north";
  }
  longitude {
    String _CoordinateAxisType "Lon";
    Float32 actual_range -97.88385, -57.23121;
    String axis "X";
    String ioos_category "Location";
    String long_name "Longitude";
    String standard_name "longitude";
    String units "degrees_east";
  }
  water_u {
    Float32 _FillValue -327.67;
    String ancillary_variables "dopx";
    String cell_methods "depth: mean time: mean";
    Float64 colorBarMaximum 0.5;
    Float64 colorBarMinimum -0.5;
    String grid_mapping "wgs84";
    String ioos_category "Currents";
    String long_name "Surface Eastward Sea Water Velocity";
    String standard_name "surface_eastward_sea_water_velocity";
    String units "m s-1";
  }
  water_v {
    Float32 _FillValue -327.67;
    String ancillary_variables "dopy";
    String cell_methods "depth: mean time: mean";
    Float64 colorBarMaximum 0.5;
    Float64 colorBarMinimum -0.5;
    String grid_mapping "wgs84";
    String ioos_category "Currents";
    String long_name "Surface Northward Sea Water Velocity";
    String standard_name "surface_northward_sea_water_velocity";
    String units "m s-1";
  }
  DOPx {
    Float32 _FillValue -327.67;
    Float64 colorBarMaximum 1.0;
    Float64 colorBarMinimum 0.0;
    String colorBarPalette "WhiteRedBlack";
    String comment "The longitudinal dilution of precision (dopx) represents the contribution of radial geometry to uncertainty in the eastward velocity estimate (u).";
    String grid_mapping "wgs84";
    String ioos_category "Currents";
    String long_name "Longitudinal Dilution of Precision";
  }
  DOPy {
    Float32 _FillValue -327.67;
    Float64 colorBarMaximum 1.0;
    Float64 colorBarMinimum 0.0;
    String colorBarPalette "WhiteRedBlack";
    String comment "The latitudinal dilution of precision (dopy) represents the contribution of radial geometry to uncertainty in the northward velocity estimate (v).";
    String grid_mapping "wgs84";
    String ioos_category "Currents";
    String long_name "Latitudinal Dilution of Precision";
  }
  hdop {
    Float32 _FillValue -327.67;
    String ancillary_variables "dopx dopy";
    Float64 colorBarMaximum 1.0;
    Float64 colorBarMinimum 0.0;
    String comment "The horizontal dilution of precision (hdop) is the vector length (magnitude) of the eastward (dopx) and northward (dopy) dilution of precision.  It represents the contribution of radial geometry to the overall uncertainty in the total velocity (u and v) estimate.";
    String ioos_category "Statistics";
    String long_name "horizontal dilution of precision";
  }
  number_of_sites {
    Byte _FillValue -127;
    String _Unsigned "false";
    String colorBarContinuous "false";
    Float64 colorBarMaximum 10.0;
    Float64 colorBarMinimum 0.0;
    String colorBarNSections "10";
    String comment "Number of radars contributing radials to the total solution";
    String ioos_category "Statistics";
    String long_name "number of contributing radars";
    String units "count";
  }
  number_of_radials {
    Int16 _FillValue -32767;
    Float64 colorBarMaximum 100.0;
    Float64 colorBarMinimum 0.0;
    String comment "Number of radials contributing to the total solution";
    String ioos_category "Statistics";
    String long_name "number of contributing radials";
    String units "count";
  }
  NC_GLOBAL {
    String cdm_data_type "Grid";
    String Conventions "COARDS, CF-1.6, ACDD-1.3";
    String creator_email "data.ioos@noaa.gov";
    String creator_name "NOAA IOOS Surface Currents Program";
    String creator_type "institution";
    String creator_url "https://ioos.noaa.gov/project/hf-radar/";
    String date_created "2026-11-08T10:30:47Z";
    String day_night_data_flag "both";
    String defaultGraphQuery "&.draw=vectors&.color=0x0000FF";
    Float64 Easternmost_Easting -57.23121;
    String format_version "2.0.00";
    Float64 geospatial_lat_max 46.49442;
    Float64 geospatial_lat_min 21.73596;
    Float64 geospatial_lat_resolution 0.05394;
    String geospatial_lat_units "degrees_north";
    Float64 geospatial_lon_max -57.23121;
    Float64 geospatial_lon_min -97.88385;
    Float64 geospatial_lon_resolution 0.0580752;
    String geospatial_lon_units "degrees_east";
    String history "HFRNet v1r0
2026-11-08T14:31:06Z https://dods.ndbc.noaa.gov/thredds/dodsC/hfradar_usegc_6km
2026-11-08T14:31:06Z https://coastwatch.pfeg.noaa.gov/erddap/griddap/ucsdHfrE6.das";
    String id "bdeb8fbf-9080-4077-ad05-286dcee21d16";
    String id_HFR "202611080700iooshfruwlsRTVusegc6km";
    String infoUrl "https://cordc.ucsd.edu/projects/mapping/";
    String institution "DOC/NOAA/NESDIS/OSPO > Office of Satellite and Product Operations, NESDIS, NOAA, U.S. Department of Commerce.";
    String instrument "OCEAN SURFACE CURRENT RADAR";
    String instrument_vocabulary "Global Change Master Directory (GCMD) Keywords, Version 21.2";
    String keywords "6km, circulation, coast, currents, dilution, Earth Science > Oceans > Ocean Circulation > Ocean Currents, east, east coast, eastward, gulf, gulf of mexico, hf radar, hourly, latitudinal, longitudinal, mexico, near, near real time, northward, ocean, oceans, radar, real, RTV, scripps, sea, seawater, surface, surface_eastward_sea_water_velocity, surface_northward_sea_water_velocity, time, velocity, water";
    String keywords_vocabulary "GCMD Science Keywords";
    String license "The data may be used and redistributed for free but is not intended
for legal use, since it may contain inaccuracies. Neither the data
Contributor, ERD, NOAA, nor the United States Government, nor any
of their employees or contractors, makes any warranty, express or
implied, including warranties of merchantability and fitness for a
particular purpose, or assumes any legal liability for the accuracy,
completeness, or usefulness, of this information.";
    String metadata_link "https://cdn.ioos.noaa.gov/media/2017/12/hfradar_nodc_sep_2014.pdf";
    String naming_authority "gov.noaa.nesdis.ncei";
    Float64 Northernmost_Northing 46.49442;
    String platform "CODAR SeaSonde, COASTAL STATIONS";
    String platform_vocabulary "Global Change Master Directory (GCMD) Keywords, Version 21.2";
    String processing_level "NOAA Level 3. Near real-time dataset with automated data acquisition and processing quality control.";
    String product_version "v1r0";
    String production_environment "INT";
    String production_site "PLAIT";
    String program "Integrated Ocean Observing System (IOOS)";
    String project "NESDIS Common Cloud Framework.";
    String publisher_email "espcoperations@noaa.gov";
    String publisher_name "DOC/NOAA/NESDIS/OSPO > Office of Satellite and Product Operations, NESDIS, NOAA, U.S. Department of Commerce.";
    String publisher_type "institution";
    String publisher_url "https://www.ospo.noaa.gov";
    String references "Terrill, E. et al., 2006. Data Management and Real-time Distribution in the HF-Radar National Network. Proceedings of the MTS/IEEE Oceans 2006 Conference, Boston MA, September 2006.";
    String source "RDLm_ASSA_2026_11_08_0700.ruv, RDLm_CEDR_2026_11_08_0700.ruv, RDLm_LISL_2026_11_08_0700.ruv, RDLm_AMAG_2026_11_08_0700.ruv, RDLm_BRAD_2026_11_08_0700.ruv, RDLm_BRIG_2026_11_08_0700.ruv, RDLm_HEMP_2026_11_08_0700.ruv, RDLm_HLGT_2026_11_08_0700.ruv, RDLm_HOOK_2026_11_08_0700.ruv, RDLm_LOVE_2026_11_08_0700.ruv, RDLm_MRCH_2026_11_08_0700.ruv, RDLm_SEAB_2026_11_08_0700.ruv, RDLm_SILD_2026_11_08_0700.ruv, RDLm_SPRK_2026_11_08_0700.ruv, RDLm_GCAP_2026_11_08_0700.ruv, RDLm_STLI_2026_11_08_0700.ruv, RDLm_BISL_2026_11_08_0700.ruv, RDLi_MISQ_2026_11_08_0700.ruv, RDLm_NWTP_2026_11_08_0645.ruv, RDLm_MCPK_2026_11_08_0700.ruv, RDLm_MOLA_2026_11_08_0700.ruv, RDLm_SMPT_2026_11_08_0700.ruv, CDN_2026_11_08_0700.ruv, NKL_2026_11_08_0700.ruv, VIR_2026_11_08_0700.ruv, RDLm_NAPL_2026_11_08_0700.ruv, RDLm_RDSR_2026_11_08_0700.ruv, RDLm_VENI_2026_11_08_0700.ruv, RDLm_WEST_2026_11_08_0700.ruv, RDLm_GPTP_2026_11_08_0700.ruv, RDLm_HBSP_2026_11_08_0700.ruv, RDLm_OBSP_2026_11_08_0700.ruv, RDLm_PCYC_2026_11_08_0700.ruv, RDLm_SGRV_2026_11_08_0700.ruv, RDLm_HAUL_2026_11_08_0700.ruv, RDLm_HILL_2026_11_08_0700.ruv, RDL_htr_2026_11_08_070005.ruv, RDL_trs_2026_11_08_070005.ruv, RDLm_FLND_2026_11_08_0700.ruv, RDLm_SUNS_2026_11_08_0700.ruv, RDLm_VIEW_2026_11_08_0700.ruv, RDLm_CAPE_2026_11_08_0700.ruv, RDLm_CMPT_2026_11_08_0700.ruv, RDLm_HLPN_2026_11_08_0700.ruv, RDLm_LEWE_2026_11_08_0700.ruv, RDLm_NANT_2026_11_08_0700.ruv, RDLm_WOOD_2026_11_08_0700.ruv, RDLm_MBNP_2026_11_08_0700.ruv, RDLm_PINS_2026_11_08_0700.ruv, RDLm_SSDE_2026_11_08_0700.ruv, RDLm_NAUS_2026_11_08_0700.ruv, RDLm_CPVN_2026_11_08_0645.ruv, RDLm_ESPT_2026_11_08_0700.ruv, RDLm_FOCF_2026_11_08_0645.ruv, RDLm_POPB_2026_11_08_0645.ruv, RDLm_TLSP_2026_11_08_0645.ruv, RDLm_OLDB_2026_11_08_0700.ruv, RDLm_PORT_2026_11_08_0700.ruv, RDL_ven_2026_11_08_070005.ruv, RDLm_PLPI_2026_11_08_0700.ruv";
    String sourceUrl "https://dods.ndbc.noaa.gov/thredds/dodsC/hfradar_usegc_6km";
    Float64 Southernmost_Northing 21.73596;
    String standard_name_vocabulary "CF Standard Name Table v70";
    String summary "Surface ocean velocities estimated from HF-Radar are representative of the upper 2.4 meters of the ocean.  The main objective of near-real time processing is to produce the best product from available data at the time of processing. Radial velocity measurements are obtained from individual radar sites through the U.S. HF-Radar Network. Hourly radial data are processed by unweighted least squares on a 6km resolution grid of the U.S. East and Gulf Coast to produce near real-time surface current maps.";
    String testOutOfDate "now-1day";
    String time_coverage_end "2026-11-08T08:00:00Z";
    String time_coverage_start "2026-08-07T01:00:00Z";
    String title "Currents, HF Radar, US East Coast and Gulf of Mexico, RTV, Near-Real Time, 2012-present, Hourly, 6km";
    Float64 Westernmost_Easting -97.88385;
  }
}
